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
import { daemon } from "@/lib/daemon";
import Link from "next/link";
import { useEffect, useState } from "react";

type Integration = { slug: string; name: string; kind: string };

export default function IntegrationsPage() {
  const [rows, setRows] = useState<Integration[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    daemon<{ integrations: Integration[] }>("/api/integrations")
      .then((body) => setRows(body.integrations))
      .catch((err: unknown) => {
        setRows([]);
        setError(err instanceof Error ? err.message : "Falha ao listar");
      });
  }, []);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Integrações</h1>
        <p className="text-muted-foreground text-sm">
          Superfície Executor: OpenAPI, GraphQL, MCP e ferramentas internas.
          Conecte uma conta para o catálogo Treg usar a sua chave.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button render={<Link href="/integracoes/explorar" />}>
            Explorar catálogo
          </Button>
        </div>
      </div>
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma integração ainda"
          description="Explore o catálogo Executor (OpenAPI, GraphQL, MCP, Google) ou conecte um provedor Treg."
        >
          <Button render={<Link href="/integracoes/explorar" />}>
            Explorar
          </Button>
        </EmptyBlock>
      ) : null}
      <div className="grid gap-3 md:grid-cols-2">
        {rows?.map((row) => (
          <Card key={row.slug}>
            <CardHeader>
              <CardTitle>{row.name}</CardTitle>
              <CardDescription>{row.slug}</CardDescription>
            </CardHeader>
            <CardContent>
              <Badge variant="secondary">{row.kind}</Badge>
            </CardContent>
            <CardFooter className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                render={<Link href={`/integracoes/${row.slug}`} />}
              >
                Detalhe
              </Button>
              <Button render={<Link href={`/conectar/${row.slug}`} />}>
                Conectar
              </Button>
            </CardFooter>
          </Card>
        ))}
      </div>
    </div>
  );
}
