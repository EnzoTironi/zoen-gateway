"use client";

import {
  CardStack,
  CardStackContent,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { PageHeader } from "@/components/page";
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
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
    <>
      <PageHeader
        title="Provedores"
        description="Só as chaves das refs. Os valores nunca saem do cofre — nem aqui, nem no MCP, nem no CLI."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma ref"
          description="Conecte uma conta. O token é gravado como SecretRef (token, api_key)."
        >
          <Link className="underline-offset-4 hover:underline" href="/conexoes">
            Ir às conexões
          </Link>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={`${row.owner}/${row.integration}/${row.name}`}
                href={`/integracoes/${row.integration}`}
                searchText={`${row.owner} ${row.integration} ${row.name} ${row.keys.join(" ")}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>
                    {row.owner}/{row.integration}/{row.name}
                  </CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {row.keys.join(", ") || "—"}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  <span className="font-mono text-[11px]">
                    {row.keys.length} chaves
                  </span>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
    </>
  );
}
