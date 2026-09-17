"use client";

import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { daemon } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type PolicyRow = {
  id: string;
  owner: string;
  pattern: string;
  action: "approve" | "require_approval" | "block";
};

function actionLabel(action: PolicyRow["action"]): string {
  switch (action) {
    case "approve":
      return "aprovar";
    case "require_approval":
      return "exigir aprovação";
    case "block":
      return "bloquear";
    default: {
      const _exhaustive: never = action;
      return _exhaustive;
    }
  }
}

export default function PoliciesPage() {
  const [rows, setRows] = useState<PolicyRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      const body = await daemon<{ policies: PolicyRow[] }>("/api/policies");
      setRows(body.policies);
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar políticas");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function remove(id: string) {
    try {
      await daemon(`/api/policies/${id}`, { method: "DELETE" });
      toast.success("Política removida");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não removeu");
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex flex-col gap-2">
          <h1 className="text-2xl font-semibold tracking-tight">Políticas</h1>
          <p className="text-muted-foreground text-sm">
            Org é a camada de fora: um `block` externo não pode ser enfraquecido
            por um `approve` interno.
          </p>
        </div>
        <Button type="button" onClick={() => setOpen(true)}>
          Nova política
        </Button>
      </div>
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma política"
          description="Sem regras, vale o padrão do plugin (`requiresApproval`). Crie um padrão como `github.*.*.repos.*`."
        >
          <Button type="button" onClick={() => setOpen(true)}>
            Criar política
          </Button>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Padrão</TableHead>
              <TableHead>Ação</TableHead>
              <TableHead>Dono</TableHead>
              <TableHead className="text-right">Ações</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id}>
                <TableCell className="font-mono text-xs">{row.pattern}</TableCell>
                <TableCell>{actionLabel(row.action)}</TableCell>
                <TableCell>{row.owner}</TableCell>
                <TableCell className="text-right">
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={() => void remove(row.id)}
                  >
                    Remover
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      <CreatePolicyDialog
        open={open}
        onOpenChange={setOpen}
        onCreated={() => {
          setOpen(false);
          void load();
        }}
      />
    </div>
  );
}

function CreatePolicyDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [pattern, setPattern] = useState("*");
  const [action, setAction] = useState<PolicyRow["action"]>("require_approval");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    try {
      await daemon("/api/policies", {
        method: "POST",
        body: JSON.stringify({ pattern, action }),
      });
      toast.success("Política criada");
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
          <DialogTitle>Nova política</DialogTitle>
          <DialogDescription>
            Padrões: `*` (tudo), `github.*` (subárvore), `github.*.*.repos.list`
            (um segmento).
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="pat">Padrão</FieldLabel>
            <Input
              id="pat"
              value={pattern}
              onChange={(event) => setPattern(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="act">Ação</FieldLabel>
            <select
              id="act"
              className="border-input h-9 rounded-md border bg-transparent px-3 text-sm"
              value={action}
              onChange={(event) =>
                setAction(event.target.value as PolicyRow["action"])
              }
            >
              <option value="approve">aprovar</option>
              <option value="require_approval">exigir aprovação</option>
              <option value="block">bloquear</option>
            </select>
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
