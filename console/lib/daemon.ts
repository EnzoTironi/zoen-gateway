const BASE = "/daemon";

let tokenPromise: Promise<string | null> | null = null;

/**
 * Lê o token de sessão do daemon local (`GET /api/console/bootstrap`).
 *
 * @returns Bearer local ou `null` se o daemon estiver fora.
 */
export async function daemonToken(): Promise<string | null> {
  tokenPromise ??= fetch(`${BASE}/api/console/bootstrap`)
    .then((res) => (res.ok ? res.json() : null))
    .then((body: { token?: string } | null) => body?.token ?? null)
    .catch(() => null);
  return tokenPromise;
}

/** Falha HTTP do daemon, com corpo JSON quando existir. */
export class DaemonError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(message: string, status: number, body: unknown) {
    super(message);
    this.name = "DaemonError";
    this.status = status;
    this.body = body;
  }
}

/**
 * Cliente JSON autenticado contra o daemon (via rewrite `/daemon`).
 *
 * @param path Caminho absoluto no daemon (`/api/catalog`).
 * @param init `fetch` init.
 */
export async function daemon<T>(path: string, init?: RequestInit): Promise<T> {
  const token = await daemonToken();
  const headers = new Headers(init?.headers);
  if (!headers.has("content-type") && init?.body) {
    headers.set("content-type", "application/json");
  }
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
    headers.set("x-treg-token", token);
    headers.set("x-executor-token", token);
  }
  const res = await fetch(`${BASE}${path}`, { ...init, headers });
  const text = await res.text();
  let json: unknown = null;
  if (text) {
    try {
      json = JSON.parse(text) as unknown;
    } catch {
      json = text;
    }
  }
  if (!res.ok) {
    const record = json as { message?: string; error?: string } | null;
    const message =
      record?.message ??
      (typeof record?.error === "string" ? record.error : null) ??
      (typeof json === "string" ? json : null) ??
      res.statusText;
    throw new DaemonError(message, res.status, json);
  }
  return json as T;
}

export type CatalogEndpoint = {
  id: string;
  capability: string;
  provider: string;
  name: string;
  summary: string;
  jobs: string[];
  method: string;
  path: string;
  access: "anonymous" | "platform" | "own_key_only";
  cost_micro: number | null;
  query: Record<string, { type?: string; required?: boolean; example?: string }>;
  routed_child?: string | null;
};

export type CatalogHit = {
  endpoint: CatalogEndpoint;
  score: number;
};

export type ConnectionRow = {
  owner: string;
  name: string;
  integration: string;
  template: string;
  address: string;
  identity_label?: string | null;
  description?: string | null;
};

export type CallOutcome = {
  status: number;
  served_via: string;
  cost_micro: number;
  child?: string;
  body: unknown;
};

export function formatMicroUsd(micro: number | null | undefined): string {
  if (micro == null) {
    return "sem preço";
  }
  if (micro === 0) {
    return "grátis";
  }
  return new Intl.NumberFormat("pt-BR", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(micro / 1_000_000);
}

export function accessLabel(access: CatalogEndpoint["access"]): string {
  switch (access) {
    case "anonymous":
      return "Anônimo";
    case "platform":
      return "Plataforma (cobrado)";
    case "own_key_only":
      return "Só com a sua chave";
    default: {
      const _exhaustive: never = access;
      return _exhaustive;
    }
  }
}

export function servedViaLabel(via: string): string {
  switch (via) {
    case "team_tool":
      return "Ferramenta da equipe";
    case "connection":
      return "Conexão (sua chave, sem cobrança)";
    case "anonymous":
      return "Anônimo";
    case "platform":
      return "Plataforma";
    case "routed":
      return "Roteado";
    default:
      return via;
  }
}
