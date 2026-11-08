import { app, t } from "@aleph-lang/sdk";

const a = app("blog");

const Post = a.entity("Post", {
  key: ["postId"],
  fields: {
    postId: t.id().generated(),
    title: t.string().min(1).max(200),
  },
});

a.route("GET /posts/:postId")
  .params({ postId: t.string().max(26) })
  .handle((_, { params }) => Post.get({ postId: params.postId }));

export default a;
