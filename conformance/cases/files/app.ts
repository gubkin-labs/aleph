/**
 * Photo uploads: a bucket of images, signed upload and download URLs, an upload handler that
 * records each photo, and deleting a photo's file. Stresses blob storage end to end.
 */
import { app, err, t } from "@aleph-lang/sdk";

const a = app("files");

const Photos = a.bucket("photos", { maxBytes: "5MB", contentTypes: ["image/*"] });

const Photo = a.entity("Photo", {
  key: ["key"],
  fields: {
    key: t.string().max(512),
    size: t
      .int()
      .min(0)
      .max(5 * 1024 * 1024),
    contentType: t.string().max(255),
    uploadedAt: t.timestamp(),
  },
});

a.route("POST /photos/:photoId/upload")
  .id("upload_photo")
  .params({ photoId: t.string().max(64) })
  .body({ contentType: t.string().max(255) })
  .handle(($, { params, body }) =>
    Photos.uploadUrl($.fmt`${$.caller}/${params.photoId}`, {
      contentType: body.contentType,
      expiresIn: "10m",
    }),
  );

a.route("GET /photos/:photoId")
  .id("get_photo")
  .params({ photoId: t.string().max(64) })
  .errors({ missing: err.notFound("No such photo") })
  .handle(($, { params, errors }) => {
    const key = $.fmt`${$.caller}/${params.photoId}`;
    const file = Photos.get(key);
    $.ensure(file.key.isSet(), errors.missing);
    return { file, download: Photos.downloadUrl(key, { expiresIn: "1h" }) };
  });

a.route("DELETE /photos/:photoId")
  .id("delete_photo")
  .params({ photoId: t.string().max(64) })
  .handle(($, { params }) => {
    const key = $.fmt`${$.caller}/${params.photoId}`;
    Photos.delete(key);
    Photo.delete({ key });
    return { deleted: key };
  });

// Every finished upload is recorded as a row, so photos can be listed and counted.
a.onUpload("recordPhoto", Photos, (_, { payload }) => {
  Photo.upsert({
    key: payload.key,
    size: payload.size,
    contentType: payload.contentType,
    uploadedAt: payload.uploadedAt,
  });
});

export default a;
