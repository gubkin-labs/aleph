/**
 * File storage: buckets of files that clients upload and download through short-lived signed
 * URLs. File bytes never pass through a plan; plans see each file's metadata, and a bucket's
 * `onUpload` handlers run when an upload finishes.
 */
import type { BucketDecl } from "@aleph-lang/manifest";
import { type Accessor, materialize, shapeOfFields } from "./data.js";
import type { ExprLike } from "./expr.js";
import { type IntField, type StringField, type TimestampField, t } from "./fields.js";
import { toNode } from "./node.js";
import { AlephBuildError, recorder } from "./recorder.js";
import { type InputValues, inputs } from "./route.js";
import { type Duration, durationMs } from "./units.js";

/** A file size: bytes, or a number with a unit (`"500KB"`, `"5MB"`, `"1GB"`; 1 KB = 1024 bytes). */
export type Size = number | `${number}${"B" | "KB" | "MB" | "GB"}`;

const UNIT_BYTES = { B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3 } as const;

/** The largest file any bucket may allow: 100 MB. */
export const MAX_FILE_BYTES: number = 100 * 1024 ** 2;

/** Converts a {@link Size} to bytes. */
export function sizeBytes(size: Size): number {
  if (typeof size === "number") return size;
  const match = /^(\d+(?:\.\d+)?)(B|KB|MB|GB)$/.exec(size);
  const unit = match?.[2] as keyof typeof UNIT_BYTES | undefined;
  if (!match?.[1] || !unit) throw new AlephBuildError(`invalid size ${JSON.stringify(size)}`);
  return Math.floor(Number(match[1]) * UNIT_BYTES[unit]);
}

/** How to declare a bucket. */
export interface BucketDef {
  /** The largest file an upload may store, like `"5MB"` (at most 100 MB). */
  maxBytes: Size;
  /** Allowed content types, exact (`"image/png"`) or by family (`"image/*"`). Omit to allow any. */
  contentTypes?: string[];
}

/** The fields of a stored file's metadata. */
export type BlobInfoFields = {
  key: StringField;
  size: IntField;
  contentType: StringField;
  uploadedAt: TimestampField;
};

/** A stored file's metadata. */
export const BLOB_INFO: BlobInfoFields = {
  key: t.string().max(512),
  size: t.int().min(0).max(MAX_FILE_BYTES),
  contentType: t.string().max(255),
  uploadedAt: t.timestamp(),
};

/** A stored file's metadata: its key, size in bytes, content type, and upload time. */
export type BlobInfo = { key: string; size: number; contentType: string; uploadedAt: number };

/** A signed upload URL: `PUT` the file's bytes to `url`, with its content type, before `expiresAt`. */
export type UploadUrl = { url: string; key: string; expiresAt: number };

/** A signed download URL: `GET` the file from `url` before `expiresAt`. */
export type DownloadUrl = { url: string; expiresAt: number };

const UPLOAD_URL = {
  url: t.string().max(8192),
  key: t.string().max(512),
  expiresAt: t.timestamp(),
};
const DOWNLOAD_URL = { url: t.string().max(8192), expiresAt: t.timestamp() };

/** How long a signed URL lasts, in whole seconds (1 second to 7 days). */
function expiresS(expiresIn: Duration | undefined): { expiresS?: number } {
  if (expiresIn === undefined) return {};
  const seconds = durationMs(expiresIn) / 1000;
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > 7 * 86_400) {
    throw new AlephBuildError(
      `a signed URL lasts 1 second to 7 days, in whole seconds; got ${JSON.stringify(expiresIn)}`,
    );
  }
  return { expiresS: seconds };
}

/** A declared bucket of files. */
export class Bucket {
  readonly name: string;
  readonly #def: BucketDecl;

  /** @internal */
  constructor(name: string, def: BucketDef) {
    const maxBytes = sizeBytes(def.maxBytes);
    if (!Number.isInteger(maxBytes) || maxBytes < 1 || maxBytes > MAX_FILE_BYTES) {
      throw new AlephBuildError(
        `bucket ${JSON.stringify(name)}: maxBytes must be 1 byte to 100MB, got ${JSON.stringify(def.maxBytes)}`,
      );
    }
    this.name = name;
    this.#def = {
      maxBytes,
      ...(def.contentTypes?.length ? { contentTypes: def.contentTypes } : {}),
    };
  }

  /** @internal */
  spec(): BucketDecl {
    return this.#def;
  }

  /**
   * A signed URL a client can upload one file to: `PUT` its bytes there with `contentType`.
   * When the upload finishes, the bucket's `onUpload` handlers run. Uploading to a key that
   * has a file replaces it. Lasts 15 minutes unless `expiresIn` says otherwise.
   */
  uploadUrl(
    key: ExprLike<string>,
    options: { contentType: ExprLike<string>; expiresIn?: Duration },
  ): Accessor<UploadUrl> {
    return materialize(
      { kind: "object", fields: shapeOfFields(UPLOAD_URL) },
      recorder().let({
        op: "uploadUrl",
        bucket: this.name,
        key: toNode(key),
        contentType: toNode(options.contentType),
        ...expiresS(options.expiresIn),
      }),
    ) as never;
  }

  /**
   * A signed URL a client can download one file from. It doesn't check the file exists: use
   * `get` for that. Lasts 15 minutes unless `expiresIn` says otherwise.
   */
  downloadUrl(
    key: ExprLike<string>,
    options: { expiresIn?: Duration } = {},
  ): Accessor<DownloadUrl> {
    return materialize(
      { kind: "object", fields: shapeOfFields(DOWNLOAD_URL) },
      recorder().let({
        op: "downloadUrl",
        bucket: this.name,
        key: toNode(key),
        ...expiresS(options.expiresIn),
      }),
    ) as never;
  }

  /** A file's metadata, or `null` if there's no such file (check `.key.isSet()`). One read. */
  get(key: ExprLike<string>): Accessor<BlobInfo | null> {
    return materialize(
      { kind: "object", fields: shapeOfFields(BLOB_INFO) },
      recorder().let({ op: "getBlob", bucket: this.name, key: toNode(key) }),
    ) as never;
  }

  /** Deletes a file: its metadata now, its bytes after commit. */
  delete(key: ExprLike<string>): void {
    recorder().let({ op: "deleteBlob", bucket: this.name, key: toNode(key) });
  }
}

/** What an `onUpload` handler gets: the uploaded file's metadata. */
export function uploadPayload(): InputValues<BlobInfoFields> {
  return inputs("payload", BLOB_INFO);
}
