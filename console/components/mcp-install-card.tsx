"use client";

import {
  CardStack,
  CardStackContent,
  CardStackHeader,
} from "@/components/card-stack";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { daemonBootstrap } from "@/lib/daemon";
import { cn } from "@/lib/utils";
import { ChevronDown, CopyIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

type Transport = "http" | "stdio";

const AGENTS = [
  { key: "cursor", label: "Cursor", mark: "Cu" },
  { key: "claude", label: "Claude", mark: "Cl" },
  { key: "opencode", label: "OpenCode", mark: "Oc" },
] as const;

function shellQuote(value: string): string {
  if (/^[A-Za-z0-9_/:=@%+.,-]+$/.test(value)) {
    return value;
  }
  return `'${value.replace(/'/g, `'\"'\"'`)}'`;
}

/**
 * Card “Conectar um agente” — mesma superfície do Executor
 * (`McpInstallCard`): CardStack + comando + Avançado + marcas dos agentes.
 */
export function McpInstallCard({ className }: { className?: string }) {
  const [mode, setMode] = useState<Transport>("http");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [origin, setOrigin] = useState("http://127.0.0.1:4788");
  const [token, setToken] = useState<string | null>(null);

  useEffect(() => {
    void daemonBootstrap().then((boot) => {
      if (boot?.origin) {
        setOrigin(boot.origin.replace(/\/$/, ""));
      }
      if (boot?.token) {
        setToken(boot.token);
      }
    });
  }, []);

  const mcpUrl = `${origin}/mcp`;
  const command =
    mode === "http"
      ? [
          "npx add-mcp",
          shellQuote(mcpUrl),
          "--transport http --name executor",
          token
            ? `--header ${shellQuote(`Authorization: Bearer ${token}`)}`
            : "",
        ]
          .filter(Boolean)
          .join(" ")
      : "npx add-mcp 'executor mcp' --name executor";

  const subtitle =
    mode === "stdio"
      ? "Exige o CLI `executor` no PATH. O stdio sobe o daemon se ele ainda não estiver no ar."
      : "Cole isto no Cursor, Claude Code ou qualquer cliente MCP. O agente passa a ver as ferramentas desta instância.";

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      toast.success("Comando copiado");
    } catch {
      toast.error("Não copiou. Selecione o comando e copie.");
    }
  }

  return (
    <CardStack className={className}>
      <CardStackHeader
        className="items-start py-4"
        rightSlot={
          <Tabs
            value={mode}
            onValueChange={(value) => setMode(value as Transport)}
          >
            <TabsList>
              <TabsTrigger value="http">HTTP remoto</TabsTrigger>
              <TabsTrigger value="stdio">Standard I/O</TabsTrigger>
            </TabsList>
          </Tabs>
        }
      >
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-foreground text-sm font-semibold">
            Conectar um agente
          </span>
          <span className="text-muted-foreground text-xs font-normal">
            {subtitle}
          </span>
        </div>
      </CardStackHeader>
      <CardStackContent>
        <div className="px-4 pt-3 pb-3">
          <div className="border-border bg-muted/25 relative overflow-hidden rounded-md border">
            <pre className="overflow-x-auto p-3 font-mono text-xs whitespace-pre-wrap">
              {command}
            </pre>
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              aria-label="Copiar comando"
              className="absolute top-2 right-2"
              onClick={() => void copy()}
            >
              <CopyIcon />
            </Button>
          </div>
          <div className="mt-3">
            <button
              type="button"
              className="text-muted-foreground hover:text-foreground flex items-center gap-1 text-xs font-medium transition-colors"
              aria-expanded={advancedOpen}
              onClick={() => setAdvancedOpen((open) => !open)}
            >
              Avançado
              <ChevronDown
                aria-hidden
                className={cn(
                  "size-3.5 transition-transform",
                  advancedOpen && "rotate-180",
                )}
              />
            </button>
            {advancedOpen ? (
              <div className="border-border bg-muted/25 mt-3 rounded-md border p-3">
                <div className="text-foreground text-xs font-medium">
                  Aprovações
                </div>
                <p className="text-muted-foreground mt-0.5 text-xs leading-5">
                  HTTP remoto abre a tela Retomar no browser. Standard I/O
                  expõe a ferramenta de resume ao modelo. O token nunca entra
                  no log do agente além deste comando de instalação.
                </p>
              </div>
            ) : null}
          </div>
        </div>
        <div className="text-muted-foreground flex items-center gap-2 px-4 py-3">
          <span className="text-xs">Funciona com o seu agente</span>
          <div className="group/agents flex items-center">
            {AGENTS.map((agent, index) => (
              <span
                key={agent.key}
                title={agent.label}
                aria-label={agent.label}
                role="img"
                style={{ zIndex: AGENTS.length - index }}
                className={cn(
                  "border-border/60 bg-background text-muted-foreground flex h-6 items-center justify-center rounded-md border px-1.5 font-mono text-[10px] transition-[margin] duration-200",
                  index > 0 && "-ml-2 group-hover/agents:ml-1",
                )}
              >
                {agent.mark}
              </span>
            ))}
          </div>
          <span className="text-xs">e outros</span>
        </div>
      </CardStackContent>
    </CardStack>
  );
}
