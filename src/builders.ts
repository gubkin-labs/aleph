import { toExpressionNode, type ExpressionInput } from "./expression.js";
import {
  validateManifest,
  type ManifestOperation,
  type ManifestRoute,
  type ManifestV1,
  type OperationStep,
  type SchemaNode,
} from "./manifest.js";
import type { SchemaBuilder } from "./schema.js";

export interface OperationSource {
  readonly id: string;
  toManifest(): ManifestOperation;
}

export interface RouteSource {
  toManifest(): ManifestRoute;
}

export class OperationBuilder implements OperationSource {
  readonly id: string;
  #input: SchemaNode | undefined;
  #output: SchemaNode | undefined;
  #steps: OperationStep[] = [];

  constructor(id: string) {
    if (id.trim().length === 0) {
      throw new RangeError("Operation id must not be empty");
    }

    this.id = id;
  }

  input<Value>(schema: SchemaBuilder<Value>): this {
    this.#input = schema.node;
    return this;
  }

  output<Value>(schema: SchemaBuilder<Value>): this {
    this.#output = schema.node;
    return this;
  }

  derive(name: string, expression: ExpressionInput): this {
    if (name.trim().length === 0) {
      throw new RangeError("Derived binding name must not be empty");
    }

    this.#steps.push({
      expression: toExpressionNode(expression),
      kind: "derive",
      name,
    });
    return this;
  }

  putRecord(
    name: string,
    collection: string,
    recordId: ExpressionInput,
    value: ExpressionInput
  ): this {
    this.#steps.push({
      collection,
      kind: "record.put",
      name,
      recordId: toExpressionNode(recordId),
      value: toExpressionNode(value),
    });
    return this;
  }

  getRecord(
    name: string,
    collection: string,
    recordId: ExpressionInput
  ): this {
    this.#steps.push({
      collection,
      kind: "record.get",
      name,
      recordId: toExpressionNode(recordId),
    });
    return this;
  }

  listRecords(name: string, collection: string, limit = 100): this {
    if (!Number.isInteger(limit) || limit < 1 || limit > 1000) {
      throw new RangeError("Record list limit must be between 1 and 1000");
    }

    this.#steps.push({
      collection,
      kind: "record.list",
      limit,
      name,
    });
    return this;
  }

  return(expression: ExpressionInput): this {
    this.#steps.push({
      expression: toExpressionNode(expression),
      kind: "return",
    });
    return this;
  }

  toManifest(): ManifestOperation {
    if (!(this.#input && this.#output)) {
      throw new Error(`Operation '${this.id}' requires input and output schemas`);
    }

    return {
      id: this.id,
      input: this.#input,
      output: this.#output,
      steps: [...this.#steps],
    };
  }
}

export class RouteBuilder implements RouteSource {
  readonly method: ManifestRoute["method"];
  readonly path: string;
  #operationId: string | undefined;

  constructor(method: ManifestRoute["method"], path: string) {
    if (!path.startsWith("/")) {
      throw new RangeError("Route path must begin with '/'");
    }

    this.method = method;
    this.path = path;
  }

  operation(source: OperationSource | string): this {
    this.#operationId = typeof source === "string" ? source : source.id;
    return this;
  }

  toManifest(): ManifestRoute {
    if (!this.#operationId) {
      throw new Error(`${this.method} ${this.path} must reference an operation`);
    }

    return {
      method: this.method,
      operationId: this.#operationId,
      path: this.path,
    };
  }
}

export class ApplicationBuilder {
  readonly appId: string;
  #operations: ManifestOperation[] = [];
  #routes: ManifestRoute[] = [];

  constructor(appId: string) {
    if (appId.trim().length === 0) {
      throw new RangeError("Application id must not be empty");
    }

    this.appId = appId;
  }

  operation(source: OperationSource | ManifestOperation): this {
    this.#operations.push(
      "toManifest" in source ? source.toManifest() : source
    );
    return this;
  }

  route(source: RouteSource | ManifestRoute): this {
    this.#routes.push("toManifest" in source ? source.toManifest() : source);
    return this;
  }

  build(): ManifestV1 {
    return validateManifest({
      appId: this.appId,
      formatVersion: 1,
      operations: [...this.#operations],
      routes: [...this.#routes],
    });
  }
}

export const application = (appId: string): ApplicationBuilder =>
  new ApplicationBuilder(appId);

export const operation = (id: string): OperationBuilder =>
  new OperationBuilder(id);

export const route = {
  delete: (path: string): RouteBuilder => new RouteBuilder("DELETE", path),
  get: (path: string): RouteBuilder => new RouteBuilder("GET", path),
  patch: (path: string): RouteBuilder => new RouteBuilder("PATCH", path),
  post: (path: string): RouteBuilder => new RouteBuilder("POST", path),
  put: (path: string): RouteBuilder => new RouteBuilder("PUT", path),
};
