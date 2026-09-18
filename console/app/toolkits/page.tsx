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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { daemon } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type ToolkitRow = {
  slug: string;
  name: string;
  connections?: string[];
};

function asToolkit(value: unknown): ToolkitRow | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const row = value as { slug?: unknown; name?: unknown; connections?: unknown };
  if (typeof row.slug !== "string") {
    return null;
  }
  return {
    slug: row.slug,
    name: typeof row.name === "string" ? row.name : row.slug,
    connections: Array.isArray(row.connections)
      ? row.connections.filter((item): item is string => typeof item === "string")
      : [],
  };
}

export default function ToolkitsPage() {
  const [rows, setRows] = useState<ToolkitRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      const body = await daemon<{ toolkits: unknown[] }>("/api/toolkits");
      setRows((body.toolkits ?? []).map(asToolkit).filter((row): row is ToolkitRow => row !== null));
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar toolkits");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function remove(slug: string) {
    try {
      await daemon(`/api/toolkits/${slug}`, { method: "DELETE" });
      toast.success("Toolkit removido");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não removeu");
    }
  }

  return (
    <>
      <PageHeader
        title="Toolkits"
        description="Superfície MCP recortada em /mcp/toolkits/:slug. Cada toolkit expõe só as conexões que você listar."
        actions={
          <Button type="button" size="sm" onClick={() => setOpen(true)}>
            Novo toolkit
          </Button>
        }
      />
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhum toolkit"
          description="Crie um recorte para um agente que não deve ver o catálogo inteiro."
        >
          <Button type="button" size="sm" onClick={() => setOpen(true)}>
            Criar toolkit
          </Button>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={row.slug}
                searchText={`${row.name} ${row.slug} ${(row.connections ?? []).join(" ")}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>{row.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>{row.slug}</CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  {(row.connections ?? []).map((connection) => (
                    <Badge key={connection} variant="outline">
                      {connection}
                    </Badge>
                  ))}
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={() => void remove(row.slug)}
                  >
                    Remover
                  </Button>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CreateToolkitDialog
        open={open}
        onOpenChange={setOpen}
        onCreated={() => {
          setOpen(false);
          void load();
        }}
      />
    </>
  );
}

function CreateToolkitDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [slug, setSlug] = useState("equipe");
  const [name, setName] = useState("Equipe");
  const [connections, setConnections] = useState("*");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    try {
      await daemon("/api/toolkits", {
        method: "POST",
        body: JSON.stringify({
          slug,
          name,
          connections: connections
            .split(",")
            .map((item) => item.trim())
            .filter(Boolean),
        }),
      });
      toast.success("Toolkit criado");
      onCreated();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não criou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Novo toolkit</DialogTitle>
          <DialogDescription>
            O slug vira o caminho /mcp/toolkits/&lt;slug&gt;. Conexões aceitam
            globs separados por vírgula (* ou github.local.work).
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="tk-slug">Slug</FieldLabel>
            <Input
              id="tk-slug"
              value={slug}
              onChange={(event) => setSlug(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="tk-name">Nome</FieldLabel>
            <Input
              id="tk-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="tk-conn">Conexões</FieldLabel>
            <Input
              id="tk-conn"
              value={connections}
              onChange={(event) => setConnections(event.target.value)}
            />
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancelar
          </Button>
          <Button type="button" onClick={() => void submit()} disabled={busy}>
            {busy ? <Spinner data-icon="inline-start" /> : null}
            Salvar
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
