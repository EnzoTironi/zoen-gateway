"use client";

import {
  CardStack,
  CardStackContent,
  CardStackEntry,
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

type ToolRow = { path: string; address: string; description: string };

export default function ToolsPage() {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<ToolRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [args, setArgs] = useState("{}");
  const [result, setResult] = useState<string>("");

  const load = useCallback(async (query: string) => {
    try {
      const suffix = query ? `?q=${encodeURIComponent(query)}` : "";
      const body = await daemon<{ tools: ToolRow[] }>(`/api/tools${suffix}`);
      setRows(body.tools);
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar ferramentas");
    }
  }, []);

  useEffect(() => {
    void load("");
  }, [load]);

  async function run() {
    try {
      const parsed = JSON.parse(args) as unknown;
      const out = await daemon<unknown>("/api/execute", {
        method: "POST",
        body: JSON.stringify({
          path: selected,
          args: parsed,
          auto_approve: true,
        }),
      });
      setResult(JSON.stringify(out, null, 2));
      toast.success("Ferramenta executada");
    } catch (err) {
      const message = err instanceof Error ? err.message : "Falha ao chamar";
      setResult(message);
      toast.error(message);
    }
  }

  return (
    <>
      <PageHeader
        title="Ferramentas"
        description="Catálogo Executor (tools.integração.dono.conexão.ferramenta). Itens bloqueados pela política não aparecem aqui."
      />
      <form
        className="mb-6 flex flex-col gap-2 sm:flex-row"
        onSubmit={(event) => {
          event.preventDefault();
          void load(q);
        }}
      >
        <Input
          value={q}
          onChange={(event) => setQ(event.target.value)}
          placeholder="Filtrar por nome"
          aria-label="Filtrar ferramentas"
        />
        <Button type="submit">Filtrar</Button>
      </form>
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma ferramenta"
          description="Adicione uma spec OpenAPI, GraphQL ou MCP, ou chame um endpoint do catálogo Treg."
        />
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack className="mb-8" searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={row.address}
                searchText={`${row.path} ${row.description}`}
                className={selected === row.path ? "bg-accent/40" : undefined}
                onClick={() => setSelected(row.path)}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle className="font-mono text-xs">
                    {row.path}
                  </CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {row.description}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CardStack>
        <CardStackContent>
          <CardStackEntryField label="Chamar ferramenta">
            <Input
              id="tool-path"
              value={selected}
              onChange={(event) => setSelected(event.target.value)}
              placeholder="caminho da ferramenta"
            />
          </CardStackEntryField>
          <CardStackEntryField label="Argumentos JSON">
            <Textarea
              value={args}
              onChange={(event) => setArgs(event.target.value)}
              aria-label="Argumentos JSON"
              className="font-mono text-xs"
              rows={6}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <Button type="button" onClick={() => void run()}>
              Executar
            </Button>
          </CardStackEntry>
        </CardStackContent>
      </CardStack>
      {result ? (
        <pre className="bg-muted mt-6 max-h-72 overflow-auto rounded-md p-3 text-xs">
          {result}
        </pre>
      ) : null}
    </>
  );
}
