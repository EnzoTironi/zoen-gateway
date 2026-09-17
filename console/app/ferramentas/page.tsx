"use client";

import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Ferramentas</h1>
        <p className="text-muted-foreground text-sm">
          Catálogo Executor (`tools.integração.dono.conexão.ferramenta`). Itens
          bloqueados pela política não aparecem aqui.
        </p>
      </div>
      <form
        className="flex flex-col gap-2 sm:flex-row"
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
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Caminho</TableHead>
              <TableHead>Descrição</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow
                key={row.address}
                data-state={selected === row.path ? "selected" : undefined}
                className="cursor-pointer"
                onClick={() => setSelected(row.path)}
              >
                <TableCell className="font-mono text-xs">{row.path}</TableCell>
                <TableCell>{row.description}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      <div className="flex flex-col gap-2">
        <label className="text-sm font-medium" htmlFor="tool-path">
          Chamar ferramenta
        </label>
        <Input
          id="tool-path"
          value={selected}
          onChange={(event) => setSelected(event.target.value)}
          placeholder="caminho da ferramenta"
        />
        <Textarea
          value={args}
          onChange={(event) => setArgs(event.target.value)}
          aria-label="Argumentos JSON"
          className="font-mono text-xs"
          rows={6}
        />
        <Button type="button" className="w-fit" onClick={() => void run()}>
          Executar
        </Button>
        {result ? (
          <pre className="bg-muted max-h-72 overflow-auto rounded-md p-3 text-xs">
            {result}
          </pre>
        ) : null}
      </div>
    </div>
  );
}
