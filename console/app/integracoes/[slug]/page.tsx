"use client";

import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">{slug}</h1>
        <p className="text-muted-foreground text-sm">
          Conexões desta integração. Segredos não são exibidos.
        </p>
      </div>
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      <div className="flex flex-wrap gap-2">
        <Button render={<Link href={`/conectar/${slug}`} />}>
          Conectar conta
        </Button>
        <Button variant="outline" render={<Link href="/integracoes" />}>
          Todas as integrações
        </Button>
      </div>
      <div className="grid gap-3">
        {rows?.map((row) => (
          <Card key={row.address}>
            <CardHeader>
              <CardTitle>{row.name}</CardTitle>
              <CardDescription>{row.address}</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-wrap gap-1.5">
              <Badge variant="outline">{row.owner}</Badge>
              <Badge variant="secondary">{row.template}</Badge>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}
