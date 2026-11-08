import { app, t } from "@aleph-lang/sdk";

const a = app("blog");

const Post = a.entity("Post", {
  key: ["postId"],
  fields: {
    postId: t.id().generated(),
    title: t.string().min(1).max(200),
    tags: t.array(t.string().max(32), 10),
  },
});

a.route("POST /posts")
  .id("create_post")
  .body({
    title: t.string().min(1).max(200),
    tags: t.array(t.string().max(32), 10),
  })
  .status(201)
  .handle((_, { body }) =>
    Post.insert({
      title: body.title,
      tags: body.tags.filter((tag) => body.tags.filter((other) => other.eq(tag)).length.eq(1)),
    }),
  );

export default a;
