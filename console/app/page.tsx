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
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Catálogo</h1>
        <p className="text-muted-foreground text-sm">
          Busque pelo <strong>trabalho</strong> que precisa fazer, não pelo
          fornecedor. Chamadas com a sua conexão não são cobradas.
        </p>
      </div>
      <form
        className="flex flex-col gap-2 sm:flex-row"
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
      <div className="grid gap-3 md:grid-cols-2">
        {hits?.map((hit) => (
          <EndpointCard
            key={hit.endpoint.id}
            endpoint={hit.endpoint}
            onCall={() => setActive(hit.endpoint)}
          />
        ))}
      </div>
      <CallDialog
        endpoint={active}
        onClose={() => setActive(null)}
      />
    </div>
  );
}

function EndpointCard({
  endpoint,
  onCall,
}: {
  endpoint: CatalogEndpoint;
  onCall: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{endpoint.name}</CardTitle>
        <CardDescription>{endpoint.summary}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <div className="flex flex-wrap gap-1.5">
          <Badge variant="secondary">{endpoint.id}</Badge>
          <Badge variant="outline">{endpoint.provider}</Badge>
          <Badge variant="outline">{accessLabel(endpoint.access)}</Badge>
          <Badge>{formatMicroUsd(endpoint.cost_micro)}</Badge>
        </div>
      </CardContent>
      <CardFooter className="flex flex-wrap gap-2">
        <Button type="button" onClick={onCall}>
          Chamar
        </Button>
        <Button variant="outline" render={<Link href={`/conectar/${endpoint.provider}`} />}>
          Conectar chave
        </Button>
      </CardFooter>
    </Card>
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
        {endpoint ? <CallForm key={endpoint.id} endpoint={endpoint} onClose={onClose} /> : null}
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
