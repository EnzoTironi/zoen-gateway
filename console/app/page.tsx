"use client";

import {
  CardStack,
  CardStackContent,
  CardStackHeader,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryMedia,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { ConnectCatalog } from "@/components/connect-catalog";
import { McpInstallCard } from "@/components/mcp-install-card";
import { PageHeader } from "@/components/page";
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { daemon } from "@/lib/daemon";
import { PlusIcon } from "lucide-react";
import { useEffect, useState } from "react";

type Integration = { slug: string; name: string; kind: string };

export default function IntegrationsHomePage() {
  const [rows, setRows] = useState<Integration[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connectOpen, setConnectOpen] = useState(false);

  useEffect(() => {
    daemon<{ integrations: Integration[] }>("/api/integrations")
      .then((body) => {
        setRows(body.integrations);
        setError(null);
      })
      .catch((err: unknown) => {
        setRows([]);
        setError(err instanceof Error ? err.message : "Falha ao listar");
      });
  }, []);

  return (
    <>
      <PageHeader
        title="Integrações"
        description="Provedores de ferramenta neste workspace. Configure uma vez, use de qualquer agente MCP."
        actions={
          <Button
            size="sm"
            className="gap-1.5"
            type="button"
            onClick={() => setConnectOpen(true)}
          >
            <PlusIcon className="size-4" />
            Conectar
          </Button>
        }
      />

      <div className="mb-8">
        <McpInstallCard />
      </div>

      <div className="border-border/50 mb-8 border-t" />

      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma integração ainda"
          description="Conecte OpenAPI, GraphQL, MCP ou um preset Google para começar a curar ferramentas."
          icon={<PlusIcon className="size-5" />}
        >
          <Button
            size="sm"
            className="gap-1.5"
            type="button"
            onClick={() => setConnectOpen(true)}
          >
            <PlusIcon className="size-4" />
            Conectar uma integração
          </Button>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <div className="mb-8">
          <CardStack searchable>
            <CardStackHeader>Workspace</CardStackHeader>
            <CardStackContent>
              {rows.map((row) => (
                <CardStackEntry
                  key={row.slug}
                  href={`/integracoes/${row.slug}`}
                  searchText={`${row.name} ${row.slug} ${row.kind}`}
                >
                  <CardStackEntryMedia>
                    {(row.name || row.slug).slice(0, 1).toUpperCase()}
                  </CardStackEntryMedia>
                  <CardStackEntryContent>
                    <CardStackEntryTitle>
                      {row.name || row.slug}
                    </CardStackEntryTitle>
                    <CardStackEntryDescription>{row.slug}</CardStackEntryDescription>
                  </CardStackEntryContent>
                  <CardStackEntryActions>
                    <Badge variant="secondary">{row.kind}</Badge>
                  </CardStackEntryActions>
                </CardStackEntry>
              ))}
            </CardStackContent>
          </CardStack>
        </div>
      ) : null}

      <Dialog open={connectOpen} onOpenChange={setConnectOpen}>
        <DialogContent className="sm:max-w-[560px]">
          <DialogHeader>
            <DialogTitle>Conectar uma integração</DialogTitle>
            <DialogDescription>
              Busque o preset ou escolha o tipo de plugin.
            </DialogDescription>
          </DialogHeader>
          <ConnectCatalog onPick={() => setConnectOpen(false)} />
        </DialogContent>
      </Dialog>
    </>
  );
}
