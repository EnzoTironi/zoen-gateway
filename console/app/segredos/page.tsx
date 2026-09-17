"use client";

import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { daemon } from "@/lib/daemon";
import Link from "next/link";
import { useEffect, useState } from "react";

type SecretRow = {
  owner: string;
  integration: string;
  name: string;
  keys: string[];
};

export default function SecretsPage() {
  const [rows, setRows] = useState<SecretRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    daemon<{ secrets: SecretRow[] }>("/api/secrets")
      .then((body) => {
        setRows(body.secrets);
        setError(null);
      })
      .catch((err: unknown) => {
        setRows([]);
        setError(err instanceof Error ? err.message : "Falha ao listar segredos");
      });
  }, []);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Segredos</h1>
        <p className="text-muted-foreground text-sm">
          Só as chaves das refs. Os valores nunca saem do cofre — nem aqui, nem
          no MCP, nem no CLI.
        </p>
      </div>
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma ref"
          description="Conecte uma conta. O token é gravado como SecretRef (`token`, `api_key`)."
        >
          <Link className="underline-offset-4 hover:underline" href="/conexoes">
            Ir às conexões
          </Link>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Conexão</TableHead>
              <TableHead>Chaves</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={`${row.owner}/${row.integration}/${row.name}`}>
                <TableCell>
                  <Link
                    className="underline-offset-4 hover:underline"
                    href={`/integracoes/${row.integration}`}
                  >
                    {row.owner}/{row.integration}/{row.name}
                  </Link>
                </TableCell>
                <TableCell className="font-mono text-xs">
                  {row.keys.join(", ") || "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
    </div>
  );
}
