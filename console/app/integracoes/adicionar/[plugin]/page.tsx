"use client";

import { ErrorBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { daemon } from "@/lib/daemon";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

type PluginKey = "openapi" | "graphql" | "mcp" | "google";

function classify(slug: string): PluginKey {
  if (slug.startsWith("google-")) {
    return "google";
  }
  if (slug === "graphql" || slug === "mcp" || slug === "openapi") {
    return slug;
  }
  return "openapi";
}

export default function AddIntegrationPage() {
  const params = useParams<{ plugin: string }>();
  const plugin = params.plugin;
  const kind = classify(plugin);
  const title = useMemo(() => {
    switch (kind) {
      case "openapi":
        return "Adicionar spec OpenAPI";
      case "graphql":
        return "Adicionar GraphQL";
      case "mcp":
        return "Adicionar servidor MCP";
      case "google":
        return `Adicionar ${plugin}`;
      default: {
        const _exhaustive: never = kind;
        return _exhaustive;
      }
    }
  }, [kind, plugin]);

  return (
    <div className="mx-auto flex w-full max-w-lg flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="text-muted-foreground text-sm">
          Isto registra uma integração Executor. Depois, conecte uma conta em
          Conexões para o catálogo Treg usar a sua chave.
        </p>
      </div>
      <AddForm plugin={plugin} kind={kind} />
      <Button variant="outline" render={<Link href="/integracoes/explorar" />}>
        Voltar à exploração
      </Button>
    </div>
  );
}

function AddForm({ plugin, kind }: { plugin: string; kind: PluginKey }) {
  const router = useRouter();
  const [slug, setSlug] = useState(plugin.replace(/^google-/, ""));
  const [name, setName] = useState(plugin);
  const [specUrl, setSpecUrl] = useState("");
  const [specJson, setSpecJson] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [mcpUrl, setMcpUrl] = useState("");
  const [command, setCommand] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (kind !== "google") {
      return;
    }
    daemon<{
      google: Array<{ id: string; url: string; name: string }>;
    }>("/api/integrations/browse")
      .then((body) => {
        const preset = body.google.find((item) => item.id === plugin);
        if (preset) {
          setSpecUrl(preset.url);
          setName(preset.name);
          setSlug(preset.id.replace(/^google-/, ""));
        }
      })
      .catch(() => {
        /* o operador ainda pode colar a URL */
      });
  }, [kind, plugin]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      switch (kind) {
        case "openapi":
        case "google": {
          let spec: unknown = specUrl
            ? { kind: "url", url: specUrl }
            : undefined;
          if (!spec && specJson.trim()) {
            spec = JSON.parse(specJson) as unknown;
          }
          if (!spec) {
            throw new Error("Informe a URL da spec ou cole o JSON.");
          }
          await daemon("/openapi/specs", {
            method: "POST",
            body: JSON.stringify({ slug, name, spec }),
          });
          break;
        }
        case "graphql":
          await daemon("/graphql/integrations", {
            method: "POST",
            body: JSON.stringify({ slug, name, endpoint }),
          });
          break;
        case "mcp":
          await daemon("/mcp/servers", {
            method: "POST",
            body: JSON.stringify({
              slug,
              name,
              url: mcpUrl || undefined,
              command: command || undefined,
            }),
          });
          break;
        default: {
          const _exhaustive: never = kind;
          return _exhaustive;
        }
      }
      toast.success(`Integração ${slug} adicionada`);
      router.push(`/integracoes/${slug}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Não adicionou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={(event) => void submit(event)}>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="slug">Slug</FieldLabel>
          <Input
            id="slug"
            value={slug}
            onChange={(event) => setSlug(event.target.value)}
            required
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="iname">Nome</FieldLabel>
          <Input
            id="iname"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </Field>
        {kind === "openapi" || kind === "google" ? (
          <>
            <Field>
              <FieldLabel htmlFor="spec-url">URL da spec / Discovery</FieldLabel>
              <Input
                id="spec-url"
                value={specUrl}
                onChange={(event) => setSpecUrl(event.target.value)}
                placeholder="https://…"
              />
            </Field>
            {kind === "openapi" ? (
              <Field>
                <FieldLabel htmlFor="spec-json">Ou JSON da spec</FieldLabel>
                <Textarea
                  id="spec-json"
                  value={specJson}
                  onChange={(event) => setSpecJson(event.target.value)}
                  className="font-mono text-xs"
                  rows={8}
                />
              </Field>
            ) : null}
          </>
        ) : null}
        {kind === "graphql" ? (
          <Field>
            <FieldLabel htmlFor="gql">Endpoint GraphQL</FieldLabel>
            <Input
              id="gql"
              value={endpoint}
              onChange={(event) => setEndpoint(event.target.value)}
              required
              placeholder="https://api.exemplo.com/graphql"
            />
          </Field>
        ) : null}
        {kind === "mcp" ? (
          <>
            <Field>
              <FieldLabel htmlFor="mcp-url">URL HTTP</FieldLabel>
              <Input
                id="mcp-url"
                value={mcpUrl}
                onChange={(event) => setMcpUrl(event.target.value)}
                placeholder="https://…"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="mcp-cmd">Ou comando stdio</FieldLabel>
              <Input
                id="mcp-cmd"
                value={command}
                onChange={(event) => setCommand(event.target.value)}
                placeholder="npx -y servidor-mcp"
              />
            </Field>
          </>
        ) : null}
      </FieldGroup>
      {error ? <ErrorBlock message={error} /> : null}
      <Button type="submit" disabled={busy}>
        {busy ? <Spinner data-icon="inline-start" /> : null}
        Registrar integração
      </Button>
    </form>
  );
}
