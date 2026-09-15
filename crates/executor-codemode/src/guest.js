"use strict";

const __invokeTool = globalThis.__executor_invokeTool;
const __log = globalThis.__executor_log;
try {
  delete globalThis.__executor_invokeTool;
} catch (e) {}
try {
  delete globalThis.__executor_log;
} catch (e) {}

const __formatLogArg = (value) => {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
};
const __formatLogLine = (args) => args.map(__formatLogArg).join(" ");
const __formatOutputText = (value) => {
  if (typeof value === "undefined") return "undefined";
  if (value === null) return "null";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
};
const __toolsEnumerationError = (path) =>
  new Error(
    (path.length === 0 ? "tools" : "tools." + path.join(".")) +
      ' is a lazy proxy and cannot be enumerated. Use tools.search({ query: "..." }) to find tools.',
  );
const __makeToolsProxy = (path) =>
  new Proxy(function () {}, {
    get(_target, prop) {
      if (prop === "then" || typeof prop === "symbol") return undefined;
      return __makeToolsProxy(path.concat(String(prop)));
    },
    ownKeys() {
      throw __toolsEnumerationError(path);
    },
    getOwnPropertyDescriptor() {
      throw __toolsEnumerationError(path);
    },
    apply(_target, _thisArg, args) {
      const toolPath = path.join(".");
      if (!toolPath) throw new Error("Tool path missing in invocation");
      return Promise.resolve(__invokeTool(toolPath, args[0])).then((raw) => {
        const msg = JSON.parse(raw);
        if (msg.paused) throw new Error("execution paused");
        if (!msg.ok) throw new Error(msg.error || "tool failed");
        return msg.value;
      });
    },
  });

const tools = __makeToolsProxy([]);
const console = {
  log: (...args) => __log("log", __formatLogLine(args)),
  warn: (...args) => __log("warn", __formatLogLine(args)),
  error: (...args) => __log("error", __formatLogLine(args)),
  info: (...args) => __log("info", __formatLogLine(args)),
  debug: (...args) => __log("debug", __formatLogLine(args)),
};
const emit = (value) => __formatOutputText(value);
const fetch = () => {
  throw new Error("fetch is disabled in QuickJS executor");
};
