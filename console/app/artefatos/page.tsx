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
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { daemon } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type Artifact = { id: string; name: string; kind: string; body: string };

export default function ArtifactsPage() {
  const [rows, setRows] = useState<Artifact[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("nota.md");
  const [body, setBody] = useState("");

  const load = useCallback(async () => {
    try {
      const out = await daemon<{ artifacts: Artifact[] }>("/api/artifacts");
      setRows(out.artifacts ?? []);
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar artefatos");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function add() {
    try {
      await daemon("/api/artifacts", {
        method: "POST",
        body: JSON.stringify({ name, kind: "markdown", body }),
      });
      toast.success("Artefato guardado");
      setBody("");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não guardou");
    }
  }

  return (
    <>
      <PageHeader
        title="Artefatos"
        description="Notas e saídas persistidas no daemon (equivalente ao /artifacts do Executor)."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {!rows && !error ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhum artefato"
          description="Guarde uma nota ou o output de uma execução."
        />
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack className="mb-8" searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={row.id}
                searchText={`${row.name} ${row.kind} ${row.body}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>{row.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {row.kind} · {row.body.slice(0, 120)}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CardStack>
        <CardStackContent>
          <CardStackEntryField label="Nome">
            <Input
              id="art-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntryField label="Conteúdo">
            <Textarea
              id="art-body"
              rows={6}
              value={body}
              onChange={(event) => setBody(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryActions>
              <Button type="button" onClick={() => void add()}>
                Guardar
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
        </CardStackContent>
      </CardStack>
    </>
  );
}
