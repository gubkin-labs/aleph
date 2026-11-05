/**
 * A project management SaaS backend: org roles and API keys, billing plans, projects with a plan
 * limit, paginated listing, tasks, watchers, counters, and bounded bulk operations.
 */
import { app, err, t } from "@aleph/sdk";

const orgId = t.string().min(1).max(64);

const a = app("projects", {
  roles: ["owner", "admin", "member"],
  apiKeyScopes: ["projects:read"],
});

const OrgBilling = a.entity("OrgBilling", {
  key: ["orgId"],
  fields: { orgId, plan: t.enum("free", "pro").default("free") },
});

const Project = a.entity("Project", {
  key: ["orgId", "projectId"],
  fields: {
    orgId,
    projectId: t.id().generated(),
    name: t.string().min(1).max(120),
    status: t.enum("active", "archived").default("active"),
    createdAt: t.timestamp().defaultNow(),
  },
  indexes: { byStatus: ["orgId", "status", "createdAt"] },
});

const Task = a.entity("Task", {
  key: ["orgId", "projectId", "taskId"],
  fields: {
    orgId,
    projectId: t.id(),
    taskId: t.id().generated(),
    title: t.string().min(1).max(200),
    done: t.bool().default(false),
  },
});

const Watcher = a.entity("Watcher", {
  key: ["orgId", "projectId", "userId"],
  fields: { orgId, projectId: t.id(), userId: t.string().max(64) },
});

const activeProjects = a.counter("activeProjects", {
  of: Project,
  groupBy: ["orgId"],
  where: (project) => project.status.eq("active"),
});

const openTasks = a.counter("openTasks", {
  of: Task,
  groupBy: ["orgId", "projectId"],
  where: (task) => task.done.not(),
});

const projectParams = { orgId, projectId: t.id() };

a.route("PUT /orgs/:orgId/billing")
  .id("setPlan")
  .params({ orgId })
  .member("orgId", { roles: ["owner"] })
  .body({ plan: t.enum("free", "pro") })
  .handle((_, { params, body }) => OrgBilling.upsert({ orgId: params.orgId, plan: body.plan }));

a.route("POST /orgs/:orgId/projects")
  .id("createProject")
  .params({ orgId })
  .member("orgId", { roles: ["owner", "admin"] })
  .body({ name: t.string().min(1).max(120) })
  .errors({ planLimit: err.forbidden("Your plan's project limit is reached") })
  .handle(($, { params, body, errors }) => {
    const billing = OrgBilling.get({ orgId: params.orgId });
    const plan = $.select(billing.exists(), billing.plan, "free");
    const maxProjects = $.match(plan, { free: 3, pro: 1000 });
    $.ensure(activeProjects.get({ orgId: params.orgId }).lt(maxProjects), errors.planLimit);
    return Project.insert({ orgId: params.orgId, name: body.name });
  });

a.route("GET /orgs/:orgId/projects")
  .id("listProjects")
  .params({ orgId })
  .member("orgId", { apiKeyScopes: ["projects:read"] })
  .query({
    status: t.enum("active", "archived").default("active"),
    limit: t.int().min(1).max(50).default(20),
    cursor: t.cursor(),
  })
  .handle((_, { params, query }) =>
    Project.page({
      index: "byStatus",
      prefix: [params.orgId, query.status],
      limit: query.limit,
      order: "desc",
      after: query.cursor,
    }),
  );

a.route("POST /orgs/:orgId/projects/:projectId/archive")
  .id("archiveProject")
  .params(projectParams)
  .member("orgId", { roles: ["owner", "admin"] })
  .errors({ notFound: err.notFound("Project not found") })
  .handle((_, { params, errors }) =>
    Project.getOrFail(params, errors.notFound).update({ status: "archived" }),
  );

a.route("POST /orgs/:orgId/projects/:projectId/tasks")
  .id("createTask")
  .params(projectParams)
  .member("orgId")
  .body({ title: t.string().min(1).max(200) })
  .errors({
    notFound: err.notFound("Project not found"),
    archived: err.conflict("Project is archived"),
  })
  .handle(($, { params, body, errors }) => {
    const project = Project.getOrFail(params, errors.notFound);
    $.ensure(project.status.eq("active"), errors.archived);
    return Task.insert({ ...params, title: body.title });
  });

a.route("POST /orgs/:orgId/projects/:projectId/tasks/complete")
  .id("completeTasks")
  .params(projectParams)
  .member("orgId")
  // 16, not 25: each completion can write the task and move it between two counter groups,
  // so 25 would exceed the 50-write route budget (the core rejects it with over_budget).
  .body({ taskIds: t.array(t.id(), 16) })
  .handle((_, { params, body }) => {
    const tasks = Task.getMany(body.taskIds.map((taskId) => ({ ...params, taskId })));
    const open = tasks.filter((task) => task.exists().and(task.done.not()));
    open.updateAll({ done: true });
    return { completed: open.length, stillOpen: openTasks.get(params) };
  });

a.route("GET /orgs/:orgId/projects/:projectId")
  .id("getProject")
  .params(projectParams)
  .member("orgId")
  .errors({ notFound: err.notFound("Project not found") })
  .handle(($, { params, errors }) => {
    const project = Project.getOrFail(params, errors.notFound);
    return {
      project,
      openTasks: openTasks.get(params),
      tasks: Task.query({ prefix: [params.orgId, params.projectId], limit: 20 }),
      summary: $.fmt`${project.name} (${project.status})`,
      myRole: $.callerRole(params.orgId),
    };
  });

a.route("POST /orgs/:orgId/projects/:projectId/watch")
  .id("watchProject")
  .params(projectParams)
  .member("orgId")
  .errors({ notFound: err.notFound("Project not found") })
  .handle(($, { params, errors }) => {
    $.ensure(Project.exists(params), errors.notFound);
    return { added: Watcher.insertIfAbsent({ ...params, userId: $.caller }) };
  });

export default a;
