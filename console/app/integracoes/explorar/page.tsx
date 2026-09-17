"use client";

import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { daemon } from "@/lib/daemon";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";

type PluginKind = { key: string; name: string; summary: string };
type GooglePreset = {
  id: string;
  name: string;
  summary: string;
  url: string;
};
type Provider = { slug: string; endpoints: number };

export default function BrowseIntegrationsPage() {
  const [plugins, setPlugins] = useState<PluginKind[] | null>(null);
  const [google, setGoogle] = useState<GooglePreset[]>([]);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useState("");

  useEffect(() => {
    daemon<{
      plugins: PluginKind[];
      google: GooglePreset[];
      providers: Provider[];
    }>("/api/integrations/browse")
      .then((body) => {
        setPlugins(body.plugins);
        setGoogle(body.google);
        setProviders(body.providers);
        setError(null);
      })
      .catch((err: unknown) => {
        setPlugins([]);
        setError(err instanceof Error ? err.message : "Falha ao explorar");
      });
  }, []);

  const needle = q.trim().toLowerCase();
  const filteredGoogle = useMemo(
    () =>
      google.filter((item) =>
        `${item.id} ${item.name} ${item.summary}`.toLowerCase().includes(needle),
      ),
    [google, needle],
  );
  const filteredProviders = useMemo(
    () =>
      providers.filter((item) => item.slug.toLowerCase().includes(needle)),
    [providers, needle],
  );

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">
          Explorar integrações
        </h1>
        <p className="text-muted-foreground text-sm">
          Superfície Executor: OpenAPI, GraphQL, MCP e presets Google Discovery.
          Os provedores Treg aparecem quando o catálogo YAML está carregado.
        </p>
      </div>
      <Input
        value={q}
        onChange={(event) => setQ(event.target.value)}
        placeholder="Filtrar por nome"
        aria-label="Filtrar catálogo de integrações"
      />
      {error ? <ErrorBlock message={error} /> : null}
      {plugins === null ? <LoadingBlock /> : null}
      {plugins && plugins.length === 0 && !error ? (
        <EmptyBlock
          title="Nada para explorar"
          description="O daemon precisa estar no ar para listar plugins e presets."
        />
      ) : null}
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-medium">Tipos de plugin</h2>
        <div className="grid gap-3 md:grid-cols-3">
          {plugins?.map((plugin) => (
            <Card key={plugin.key}>
              <CardHeader>
                <CardTitle>{plugin.name}</CardTitle>
                <CardDescription>{plugin.summary}</CardDescription>
              </CardHeader>
              <CardFooter>
                <Button
                  render={<Link href={`/integracoes/adicionar/${plugin.key}`} />}
                >
                  Adicionar
                </Button>
              </CardFooter>
            </Card>
          ))}
        </div>
      </section>
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-medium">Google Discovery</h2>
        <div className="grid gap-3 md:grid-cols-2">
          {filteredGoogle.map((preset) => (
            <Card key={preset.id}>
              <CardHeader>
                <CardTitle>{preset.name}</CardTitle>
                <CardDescription>{preset.summary}</CardDescription>
              </CardHeader>
              <CardFooter>
                <Button
                  render={
                    <Link href={`/integracoes/adicionar/${preset.id}`} />
                  }
                >
                  Adicionar
                </Button>
              </CardFooter>
            </Card>
          ))}
        </div>
      </section>
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-medium">Provedores do catálogo Treg</h2>
        {filteredProviders.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            Nenhum provedor além do seed. Defina `EXECUTOR_CATALOG_DIR` para
            ingerir os YAML do Treg.
          </p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {filteredProviders.map((provider) => (
              <Button
                key={provider.slug}
                variant="outline"
                size="sm"
                render={<Link href={`/conectar/${provider.slug}`} />}
              >
                {provider.slug}
                <Badge variant="secondary">{provider.endpoints}</Badge>
              </Button>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
