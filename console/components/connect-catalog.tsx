"use client";

import {
  CardStack,
  CardStackContent,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryMedia,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Badge } from "@/components/ui/badge";
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

/**
 * Catálogo de conexão — plugins, presets Google e provedores Treg.
 * Usado no diálogo da home e na página Explorar.
 */
export function ConnectCatalog({ onPick }: { onPick?: () => void }) {
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
    () => providers.filter((item) => item.slug.toLowerCase().includes(needle)),
    [needle, providers],
  );

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <Input
        value={q}
        onChange={(event) => setQ(event.target.value)}
        placeholder="Buscar ou filtrar por nome…"
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
      {plugins && plugins.length > 0 ? (
        <section className="flex min-w-0 flex-col gap-2">
          <p className="text-foreground/80 text-xs font-medium">
            Ou adicione manualmente
          </p>
          <CardStack>
            <CardStackContent>
              {plugins.map((plugin) => (
                <CardStackEntry
                  key={plugin.key}
                  href={`/integracoes/adicionar/${plugin.key}`}
                  searchText={`${plugin.name} ${plugin.summary}`}
                  onClick={onPick}
                >
                  <CardStackEntryMedia>
                    {plugin.name.slice(0, 1)}
                  </CardStackEntryMedia>
                  <CardStackEntryContent>
                    <CardStackEntryTitle>{plugin.name}</CardStackEntryTitle>
                    <CardStackEntryDescription>
                      {plugin.summary}
                    </CardStackEntryDescription>
                  </CardStackEntryContent>
                </CardStackEntry>
              ))}
            </CardStackContent>
          </CardStack>
        </section>
      ) : null}
      {filteredGoogle.length > 0 ? (
        <section className="flex min-w-0 flex-col gap-2">
          <p className="text-foreground/80 text-xs font-medium">
            Integrações populares
          </p>
          <CardStack>
            <CardStackContent className="max-h-64 overflow-y-auto">
              {filteredGoogle.map((preset) => (
                <CardStackEntry
                  key={preset.id}
                  href={`/integracoes/adicionar/${preset.id}`}
                  searchText={`${preset.name} ${preset.summary}`}
                  onClick={onPick}
                >
                  <CardStackEntryMedia>
                    {preset.name.slice(0, 1)}
                  </CardStackEntryMedia>
                  <CardStackEntryContent>
                    <CardStackEntryTitle>{preset.name}</CardStackEntryTitle>
                    <CardStackEntryDescription>
                      {preset.summary}
                    </CardStackEntryDescription>
                  </CardStackEntryContent>
                  <CardStackEntryActions>
                    <Badge variant="secondary">Google</Badge>
                  </CardStackEntryActions>
                </CardStackEntry>
              ))}
            </CardStackContent>
          </CardStack>
        </section>
      ) : null}
      <section className="flex min-w-0 flex-col gap-2">
        <p className="text-foreground/80 text-xs font-medium">
          Provedores do catálogo Treg
        </p>
        {filteredProviders.length === 0 ? (
          <p className="text-muted-foreground text-xs">
            Nenhum provedor além do seed. Defina `EXECUTOR_CATALOG_DIR` para
            ingerir os YAML do Treg.
          </p>
        ) : (
          <CardStack>
            <CardStackContent>
              {filteredProviders.map((provider) => (
                <CardStackEntry
                  key={provider.slug}
                  href={`/conectar/${provider.slug}`}
                  searchText={provider.slug}
                  onClick={onPick}
                >
                  <CardStackEntryMedia>
                    {provider.slug.slice(0, 1)}
                  </CardStackEntryMedia>
                  <CardStackEntryContent>
                    <CardStackEntryTitle>{provider.slug}</CardStackEntryTitle>
                    <CardStackEntryDescription>
                      {provider.endpoints} endpoints
                    </CardStackEntryDescription>
                  </CardStackEntryContent>
                </CardStackEntry>
              ))}
            </CardStackContent>
          </CardStack>
        )}
      </section>
    </div>
  );
}
