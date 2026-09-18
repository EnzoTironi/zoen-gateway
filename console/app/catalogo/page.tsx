"use client";

import {
  CardStack,
  CardStackContent,
  CardStackHeader,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryMedia,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { PageHeader } from "@/components/page";
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  accessLabel,
  daemon,
  DaemonError,
  formatMicroUsd,
  servedViaLabel,
  type CallOutcome,
  type CatalogEndpoint,
  type CatalogHit,
} from "@/lib/daemon";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

export default function CatalogPage() {
  const [query, setQuery] = useState("encontrar e-mail");
  const [hits, setHits] = useState<CatalogHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [active, setActive] = useState<CatalogEndpoint | null>(null);

  const search = useCallback(async (q: string) => {
    try {
      const encoded = encodeURIComponent(q);
      const body = await daemon<{ items: CatalogHit[] }>(
        `/api/catalog?q=${encoded}`,
      );
      setHits(body.items);
      setError(null);
    } catch (err) {
      setHits([]);
      setError(err instanceof Error ? err.message : "Falha na busca");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void search(query);
    // primeira carga com o trabalho de exemplo
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search]);

  return (
    <>
      <PageHeader
        title="Catálogo"
        description="Superfície Treg: busque pelo trabalho, não pelo fornecedor. Chamadas com a sua conexão não são cobradas."
      />
      <form
        className="mb-6 flex flex-col gap-2 sm:flex-row"
        onSubmit={(event) => {
          event.preventDefault();
          void search(query);
        }}
      >
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="ex.: encontrar e-mail, backlinks, usuário github"
          aria-label="Buscar por trabalho"
        />
        <Button type="submit" disabled={loading}>
          {loading ? <Spinner data-icon="inline-start" /> : null}
          Buscar
        </Button>
      </form>
      {error ? <ErrorBlock message={error} /> : null}
      {loading && !hits ? <LoadingBlock /> : null}
      {!loading && hits && hits.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhum endpoint"
          description="Tente outro trabalho, em português ou inglês. Com EXECUTOR_CATALOG_DIR o daemon ingere os YAML do Treg (seed vence nos ids já curados)."
        />
      ) : null}
      {hits && hits.length > 0 ? (
        <CardStack searchable>
          <CardStackHeader>Endpoints</CardStackHeader>
          <CardStackContent>
            {hits.map((hit) => (
              <CardStackEntry
                key={hit.endpoint.id}
                searchText={`${hit.endpoint.name} ${hit.endpoint.id} ${hit.endpoint.provider} ${hit.endpoint.summary}`}
              >
                <CardStackEntryMedia>
                  {hit.endpoint.provider.slice(0, 1).toUpperCase()}
                </CardStackEntryMedia>
                <CardStackEntryContent>
                  <CardStackEntryTitle>{hit.endpoint.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {hit.endpoint.summary}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  <Badge variant="outline">{hit.endpoint.provider}</Badge>
                  <Badge variant="secondary">
                    {accessLabel(hit.endpoint.access)}
                  </Badge>
                  <Badge>{formatMicroUsd(hit.endpoint.cost_micro)}</Badge>
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => setActive(hit.endpoint)}
                  >
                    Chamar
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    nativeButton={false}
                    render={<Link href={`/conectar/${hit.endpoint.provider}`} />}
                  >
                    Chave
                  </Button>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CallDialog endpoint={active} onClose={() => setActive(null)} />
    </>
  );
}

function CallDialog({
  endpoint,
  onClose,
}: {
  endpoint: CatalogEndpoint | null;
  onClose: () => void;
}) {
  return (
    <Dialog open={endpoint !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        {endpoint ? (
          <CallForm key={endpoint.id} endpoint={endpoint} onClose={onClose} />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function initialQuery(endpoint: CatalogEndpoint): Record<string, string> {
  const next: Record<string, string> = {};
  for (const [key, spec] of Object.entries(endpoint.query ?? {})) {
    if (spec?.example) {
      next[key] = spec.example;
    }
  }
  return next;
}

function CallForm({
  endpoint,
  onClose,
}: {
  endpoint: CatalogEndpoint;
  onClose: () => void;
}) {
  const fields = useMemo(() => {
    if (!endpoint.query || Array.isArray(endpoint.query)) {
      return [] as Array<[string, { required?: boolean; example?: string }]>;
    }
    return Object.entries(endpoint.query);
  }, [endpoint]);
  const [values, setValues] = useState<Record<string, string>>(() =>
    initialQuery(endpoint),
  );
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CallOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const out = await daemon<CallOutcome>("/api/call", {
        method: "POST",
        body: JSON.stringify({ id: endpoint.id, query: values }),
      });
      setResult(out);
      toast.success(
        `Atendido via ${servedViaLabel(out.served_via)} · ${formatMicroUsd(out.cost_micro)}`,
      );
    } catch (err) {
      if (err instanceof DaemonError && err.status === 402) {
        const body = err.body as {
          balance_micro?: number;
          estimated_cost_micro?: number;
        };
        setError(
          `Saldo insuficiente (${formatMicroUsd(body.balance_micro ?? 0)}). Esta chamada custaria ${formatMicroUsd(body.estimated_cost_micro ?? 0)}. Recarregue em Saldo ou conecte a sua chave.`,
        );
      } else if (err instanceof DaemonError && err.status === 403) {
        setError(
          err.message ||
            "Conecte a sua chave para este provedor. A plataforma não publica preço.",
        );
      } else {
        setError(err instanceof Error ? err.message : "Falha na chamada");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{endpoint.name}</DialogTitle>
        <DialogDescription>
          {`${endpoint.id} · ${formatMicroUsd(endpoint.cost_micro)} · ${accessLabel(endpoint.access)}`}
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        {fields.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            Este endpoint não declara parâmetros.
          </p>
        ) : (
          fields.map(([name, spec]) => (
            <Field key={name} data-invalid={!values[name] && spec.required}>
              <FieldLabel htmlFor={`q-${name}`}>
                {name}
                {spec.required ? " *" : ""}
              </FieldLabel>
              <Input
                id={`q-${name}`}
                value={values[name] ?? ""}
                required={spec.required}
                aria-invalid={!values[name] && spec.required}
                onChange={(event) =>
                  setValues((prev) => ({ ...prev, [name]: event.target.value }))
                }
              />
            </Field>
          ))
        )}
      </FieldGroup>
      {error ? <ErrorBlock message={error} /> : null}
      {result ? (
        <pre className="bg-muted max-h-56 overflow-auto rounded-md p-3 text-xs">
          {JSON.stringify(result, null, 2)}
        </pre>
      ) : null}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Fechar
        </Button>
        <Button type="button" onClick={() => void submit()} disabled={busy}>
          {busy ? <Spinner data-icon="inline-start" /> : null}
          Executar
        </Button>
      </DialogFooter>
    </>
  );
}
