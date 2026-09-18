"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { daemon } from "@/lib/daemon";
import { cn } from "@/lib/utils";
import { PlusIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";

type Integration = { slug: string; name: string; kind: string };

type Command = {
  id: string;
  group: string;
  label: string;
  hint?: string;
  href: string;
};

const PAGES: Command[] = [
  { id: "home", group: "Ir para", label: "Integrações", href: "/" },
  {
    id: "explore",
    group: "Ir para",
    label: "Explorar integrações",
    href: "/integracoes/explorar",
  },
  { id: "secrets", group: "Ir para", label: "Provedores", href: "/segredos" },
  { id: "policies", group: "Ir para", label: "Políticas", href: "/politicas" },
  { id: "toolkits", group: "Ir para", label: "Toolkits", href: "/toolkits" },
  { id: "artifacts", group: "Ir para", label: "Artefatos", href: "/artefatos" },
  { id: "catalog", group: "Ir para", label: "Catálogo", href: "/catalogo" },
  { id: "balance", group: "Ir para", label: "Saldo", href: "/saldo" },
  { id: "arena", group: "Ir para", label: "Arena", href: "/arena" },
  { id: "orgs", group: "Ir para", label: "Equipes", href: "/equipes" },
  { id: "skills", group: "Ir para", label: "Skills", href: "/skills" },
  { id: "tools", group: "Ir para", label: "Ferramentas", href: "/ferramentas" },
  { id: "connections", group: "Ir para", label: "Conexões", href: "/conexoes" },
];

const ADD: Command[] = [
  {
    id: "add-openapi",
    group: "Adicionar",
    label: "Adicionar OpenAPI",
    href: "/integracoes/adicionar/openapi",
  },
  {
    id: "add-graphql",
    group: "Adicionar",
    label: "Adicionar GraphQL",
    href: "/integracoes/adicionar/graphql",
  },
  {
    id: "add-mcp",
    group: "Adicionar",
    label: "Adicionar MCP",
    href: "/integracoes/adicionar/mcp",
  },
];

/**
 * Paleta ⌘K — navega páginas, integrações ligadas e atalhos de adicionar.
 */
export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const router = useRouter();
  const [query, setQuery] = useState("");
  const [rows, setRows] = useState<Integration[]>([]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        onOpenChange(!open);
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onOpenChange, open]);

  useEffect(() => {
    if (!open) {
      setQuery("");
      return;
    }
    daemon<{ integrations: Integration[] }>("/api/integrations")
      .then((body) => setRows(body.integrations ?? []))
      .catch(() => setRows([]));
  }, [open]);

  const connected: Command[] = useMemo(
    () =>
      rows.map((row) => ({
        id: `int-${row.slug}`,
        group: "Conectadas",
        label: row.name || row.slug,
        hint: row.kind,
        href: `/integracoes/${row.slug}`,
      })),
    [rows],
  );

  const needle = query.trim().toLowerCase();
  const groups = useMemo(() => {
    const all = [...connected, ...ADD, ...PAGES];
    const filtered = needle
      ? all.filter((item) =>
          `${item.group} ${item.label} ${item.hint ?? ""} ${item.href}`
            .toLowerCase()
            .includes(needle),
        )
      : all;
    const order = ["Conectadas", "Adicionar", "Ir para"];
    return order
      .map((name) => ({
        name,
        items: filtered.filter((item) => item.group === name),
      }))
      .filter((group) => group.items.length > 0);
  }, [connected, needle]);

  const go = useCallback(
    (href: string) => {
      onOpenChange(false);
      router.push(href);
    },
    [onOpenChange, router],
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="gap-0 overflow-hidden p-0 sm:max-w-lg"
      >
        <DialogTitle className="sr-only">Comandos</DialogTitle>
        <DialogDescription className="sr-only">
          Busque páginas, integrações ou atalhos de adicionar.
        </DialogDescription>
        <div className="border-b px-3 py-2">
          <Input
            autoFocus
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Buscar integrações ou saltar…"
            aria-label="Buscar comandos"
            className="h-9 border-0 bg-transparent shadow-none focus-visible:ring-0"
          />
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          {groups.length === 0 ? (
            <p className="text-muted-foreground px-3 py-6 text-center text-sm">
              Nenhum resultado.
            </p>
          ) : (
            groups.map((group) => (
              <div key={group.name} className="mb-1">
                <p className="text-muted-foreground px-2.5 py-1.5 text-[11px] font-medium tracking-wide uppercase">
                  {group.name}
                </p>
                {group.items.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => go(item.href)}
                    className={cn(
                      "hover:bg-accent/40 flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm",
                    )}
                  >
                    {item.group === "Adicionar" ? (
                      <PlusIcon className="text-muted-foreground size-3.5" />
                    ) : null}
                    <span className="min-w-0 flex-1 truncate">{item.label}</span>
                    {item.hint ? (
                      <span className="text-muted-foreground font-mono text-[11px]">
                        {item.hint}
                      </span>
                    ) : null}
                  </button>
                ))}
              </div>
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
