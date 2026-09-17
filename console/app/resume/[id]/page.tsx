"use client";

import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { daemon } from "@/lib/daemon";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type ExecutionWire = {
  executionId: string;
  state: unknown;
};

function pauseInfo(state: unknown): {
  kind: string;
  message: string;
  url?: string;
} {
  if (state === "Running") {
    return { kind: "em execução", message: "Ainda em andamento." };
  }
  if (!state || typeof state !== "object") {
    return { kind: "desconhecido", message: "Execução sem detalhes." };
  }
  const record = state as Record<string, unknown>;
  if ("Completed" in record) {
    return { kind: "concluída", message: "Esta execução já terminou." };
  }
  if ("Failed" in record) {
    return { kind: "falhou", message: "A execução falhou." };
  }
  if ("Paused" in record && record.Paused && typeof record.Paused === "object") {
    const paused = record.Paused as {
      execution?: { reason?: Record<string, unknown> };
    };
    const reason = paused.execution?.reason ?? {};
    const kind = String(reason.kind ?? "pausada");
    switch (kind) {
      case "approval":
        return {
          kind: "aprovação",
          message:
            String(reason.description ?? "") ||
            `A política pede aprovação para ${String(reason.address ?? "esta ferramenta")}.`,
        };
      case "auth":
        return {
          kind: "oauth",
          message: String(reason.message ?? "Autentique-se para continuar."),
          url: typeof reason.url === "string" ? reason.url : undefined,
        };
      case "elicitation":
        return {
          kind: "elicitação",
          message: String(reason.message ?? "Preencha o formulário para continuar."),
        };
      default:
        return { kind, message: "A execução está pausada." };
    }
  }
  return { kind: "estado", message: JSON.stringify(state) };
}

export default function ResumePage() {
  const params = useParams<{ id: string }>();
  const id = params.id;
  const [data, setData] = useState<ExecutionWire | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [persist, setPersist] = useState("session");
  const [content, setContent] = useState("{}");
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<string>("");

  const load = useCallback(async () => {
    try {
      const body = await daemon<ExecutionWire>(`/executions/${id}`);
      setData(body);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Execução não encontrada");
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const info = data ? pauseInfo(data.state) : null;

  async function act(action: "accept" | "decline" | "cancel") {
    setBusy(action);
    try {
      let parsed: unknown;
      try {
        parsed = JSON.parse(content) as unknown;
      } catch {
        parsed = content;
      }
      const out = await daemon<unknown>(`/executions/${id}/resume`, {
        method: "POST",
        body: JSON.stringify({
          action,
          persist,
          content: parsed,
        }),
      });
      setResult(JSON.stringify(out, null, 2));
      toast.success(
        action === "accept"
          ? "Aprovado"
          : action === "decline"
            ? "Recusado"
            : "Cancelado",
      );
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Falha ao retomar");
    } finally {
      setBusy(null);
    }
  }

  if (error) {
    return <ErrorBlock title="Não deu para abrir a pausa" message={error} />;
  }
  if (!data || !info) {
    return <LoadingBlock label="Carregando execução pausada…" />;
  }

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Retomar</h1>
        <p className="text-muted-foreground text-sm">
          Aprovação, OAuth ou elicitação — use os botões abaixo. Não é só um
          link impresso no terminal.
        </p>
      </div>
      <div className="flex flex-col gap-1 rounded-lg border p-4">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {info.kind}
        </p>
        <p>{info.message}</p>
        <p className="font-mono text-xs text-muted-foreground">{id}</p>
        {info.url ? (
          <a
            className="text-sm underline underline-offset-4"
            href={info.url}
            target="_blank"
            rel="noreferrer"
          >
            Abrir login do provedor
          </a>
        ) : null}
      </div>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="persist">Lembrar desta escolha</FieldLabel>
          <Input
            id="persist"
            value={persist}
            onChange={(event) => setPersist(event.target.value)}
            placeholder="session ou always"
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="content">Conteúdo (JSON, se houver formulário)</FieldLabel>
          <Textarea
            id="content"
            value={content}
            onChange={(event) => setContent(event.target.value)}
            className="font-mono text-xs"
            rows={4}
          />
        </Field>
      </FieldGroup>
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={() => void act("accept")}
          disabled={busy !== null}
        >
          {busy === "accept" ? <Spinner data-icon="inline-start" /> : null}
          Aprovar
        </Button>
        <Button
          type="button"
          variant="destructive"
          onClick={() => void act("decline")}
          disabled={busy !== null}
        >
          Recusar
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => void act("cancel")}
          disabled={busy !== null}
        >
          Cancelar
        </Button>
      </div>
      {result ? (
        <pre className="bg-muted max-h-72 overflow-auto rounded-md p-3 text-xs">
          {result}
        </pre>
      ) : null}
    </div>
  );
}
