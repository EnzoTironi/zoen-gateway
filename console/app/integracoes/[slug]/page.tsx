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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { daemon, type ConnectionRow } from "@/lib/daemon";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useEffect, useState } from "react";

export default function IntegrationDetailPage() {
  const params = useParams<{ slug: string }>();
  const slug = params.slug;
  const [rows, setRows] = useState<ConnectionRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    daemon<{ connections: ConnectionRow[] }>("/api/connections")
      .then((body) =>
        setRows(body.connections.filter((c) => c.integration === slug)),
      )
      .catch((err: unknown) => {
        setRows([]);
        setError(err instanceof Error ? err.message : "Falha ao carregar");
      });
  }, [slug]);

  return (
    <>
      <PageHeader
        title={slug}
        description="Conexões desta integração. Segredos não são exibidos."
        actions={
          <Button size="sm" nativeButton={false} render={<Link href={`/conectar/${slug}`} />}>
            Conectar conta
          </Button>
        }
      />
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma conexão"
          description="Salve um token ou inicie OAuth. Os valores nunca voltam na listagem."
        >
          <Button size="sm" nativeButton={false} render={<Link href={`/conectar/${slug}`} />}>
            Conectar conta
          </Button>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={row.address}
                searchText={`${row.name} ${row.address} ${row.owner}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>{row.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>{row.address}</CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  <Badge variant="outline">{row.owner}</Badge>
                  <Badge variant="secondary">{row.template}</Badge>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
    </>
  );
}
