// A deliberately invalid app: the loop can write 100 rows, over the 50-write route limit.
import { $, type App, app, t } from "@aleph/sdk";

const a: App<never, never> = app("overBudget");
const Item = a.entity("Item", {
  key: ["id"],
  fields: { id: t.id().generated(), name: t.string().max(50) },
});

a.route("POST /items")
  .auth("public")
  .body({ names: t.array(t.string().max(50), 100) })
  .handle((_, { body }) => {
    $.forEach(body.names, (name) => {
      Item.insert({ name });
    });
    return null;
  });

export default a;
