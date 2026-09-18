"use client";

import {
  CardStack,
  CardStackContent,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryField,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { PageHeader } from "@/components/page";
import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { daemon, formatMicroUsd } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type Capability = {
  capability: string;
  endpoints: {
    id: string;
    provider: string;
    name: string;
    cost_micro: number | null;
  }[];
};

type ArenaResult = {
  id: string;
  provider: string;
  name: string;
  status?: number;
  cost_micro?: number;
  ms?: number;
  error?: string;
};

export default function ArenaPage() {
  const [caps, setCaps] = useState<Capability[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [capability, setCapability] = useState("people.email.find");
  const [domain, setDomain] = useState("stripe.com");
  const [name, setName] = useState("Patrick Collison");
  const [results, setResults] = useState<ArenaResult[] | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const body = await daemon<{ capabilities: Capability[] }>(
        "/api/arena/capabilities",
      );
      setCaps(body.capabilities ?? []);
      setError(null);
    } catch (err) {
      setCaps([]);
      setError(err instanceof Error ? err.message : "Falha ao listar a Arena");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function run() {
    setBusy(true);
    try {
      const out = await daemon<{ results: ArenaResult[] }>("/api/arena/run", {
        method: "POST",
        body: JSON.stringify({
          capability,
          query: { domain, full_name: name },
        }),
      });
      setResults(out.results ?? []);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não correu");
    } finally {
      setBusy(false);
    }
  }

  async function vote(winnerId: string) {
    try {
      await daemon("/api/arena/votes", {
        method: "POST",
        body: JSON.stringify({ capability, winner_id: winnerId }),
      });
      toast.success("Voto registrado");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não votou");
    }
  }

  return (
    <>
      <PageHeader
        title="Enrich Arena"
        description="Compare provedores da mesma capacidade. Chamadas cobráveis usam o saldo da equipe."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {!caps && !error ? <LoadingBlock /> : null}
      <CardStack className="mb-8">
        <CardStackContent>
          <CardStackEntryField label="Capacidade">
            <Input
              aria-label="Capacidade"
              value={capability}
              onChange={(event) => setCapability(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntryField label="Domínio">
            <Input
              aria-label="Domínio"
              value={domain}
              onChange={(event) => setDomain(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntryField label="Nome">
            <Input
              aria-label="Nome"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryContent>
              <CardStackEntryDescription>
                Capacidades com dois ou mais concorrentes:{" "}
                {caps?.map((item) => item.capability).join(", ") || "nenhuma"}
              </CardStackEntryDescription>
            </CardStackEntryContent>
            <CardStackEntryActions>
              <Button type="button" onClick={() => void run()} disabled={busy}>
                {busy ? <Spinner data-icon="inline-start" /> : null}
                Correr waterfall
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
        </CardStackContent>
      </CardStack>
      {results ? (
        <CardStack>
          <CardStackContent>
            {results.map((row) => (
              <CardStackEntry key={row.id}>
                <CardStackEntryContent>
                  <CardStackEntryTitle>{row.provider}</CardStackEntryTitle>
                  <CardStackEntryDescription>{row.id}</CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  <span className="font-mono text-xs">
                    {formatMicroUsd(row.cost_micro ?? 0)}
                  </span>
                  <span className="font-mono text-xs">{row.ms ?? "—"} ms</span>
                  <span className="text-xs">{row.error ?? row.status}</span>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => void vote(row.id)}
                  >
                    Votar
                  </Button>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
    </>
  );
}
