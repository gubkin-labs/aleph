/** `@aleph/cli`: the commands behind the `aleph` binary, usable programmatically. */
export { check, formatDiagnostics, formatEvolution, formatReport } from "./check.js";
export { generateClient, inputType, schemaType } from "./client.js";
export {
  type Activity,
  callerFromHeaders,
  type DevServerOptions,
  type MockResponse,
  type SeedData,
  startDevServer,
} from "./dev.js";
export { LoadError, loadManifest, loadModule } from "./load.js";
export { main } from "./main.js";
export { fieldSchema, mcpTools, openapi, responseSchema } from "./openapi.js";
