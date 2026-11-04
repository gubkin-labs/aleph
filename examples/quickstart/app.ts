import {
  application,
  arg,
  operation,
  ref,
  route,
  schema,
} from "aleph-language";

const scaleValue = operation("numbers.scale")
  .input(
    schema.object({
      value: schema.number(),
    })
  )
  .output(
    schema.object({
      value: schema.number(),
    })
  )
  .derive("scaled", arg("value").add(2).multiply(100))
  .return({ value: ref("scaled") });

const createNote = operation("notes.create")
  .input(
    schema.object({
      id: schema.uuid(),
      title: schema.string().max(120),
    })
  )
  .output(
    schema.object({
      id: schema.uuid(),
      title: schema.string(),
    })
  )
  .putRecord("note", "notes", arg("id"), {
    id: arg("id"),
    title: arg("title"),
  })
  .return(ref("note"));

export const manifest = application("quickstart")
  .operation(scaleValue)
  .operation(createNote)
  .route(route.post("/numbers/scale").operation(scaleValue))
  .route(route.post("/notes").operation(createNote))
  .build();
