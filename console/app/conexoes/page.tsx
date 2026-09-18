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
import { daemon, type ConnectionRow } from "@/lib/daemon";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

export default function ConnectionsPage() {
  const [rows, setRows] = useState<ConnectionRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      const body = await daemon<{ connections: ConnectionRow[] }>(
        "/api/connections",
      );
      setRows(body.connections);
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar conexões");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function remove(row: ConnectionRow) {
    try {
      await daemon(
        `/api/connections/${row.owner}/${row.integration}/${row.name}`,
        { method: "DELETE" },
      );
      toast.success("Conexão removida");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não removeu");
    }
  }

  async function refresh(row: ConnectionRow) {
    try {
      await daemon(
        `/api/connections/${row.owner}/${row.integration}/${row.name}/validate`,
        { method: "POST" },
      );
      toast.success("Conexão validada");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não validou");
    }
  }

  return (
    <>
      <PageHeader
        title="Conexões"
        description="Credenciais nascem ligadas a uma integração. Os valores nunca voltam na listagem."
        actions={
          <Button type="button" size="sm" onClick={() => setOpen(true)}>
            Nova conexão
          </Button>
        }
      />
      {error ? <ErrorBlock message={error} /> : null}
      {rows === null ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma conexão"
          description="Salve um token ou chave de API. Com a sua chave, o catálogo não cobra micro-USD."
        >
          <Button type="button" size="sm" onClick={() => setOpen(true)}>
            Conectar conta
          </Button>
        </EmptyBlock>
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={`${row.owner}/${row.integration}/${row.name}`}
                searchText={`${row.owner} ${row.integration} ${row.name} ${row.template}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>{row.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {row.owner}/{row.integration} · {row.template}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
                <CardStackEntryActions>
                  <Button
                    variant="outline"
                    size="sm"
                    nativeButton={false}
                    render={<Link href={`/integracoes/${row.integration}`} />}
                  >
                    Abrir
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => void refresh(row)}
                  >
                    Validar
                  </Button>
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={() => void remove(row)}
                  >
                    Remover
                  </Button>
                </CardStackEntryActions>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CreateConnectionDialog
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

function CreateConnectionDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [integration, setIntegration] = useState("hunter");
  const [name, setName] = useState("work");
  const [template, setTemplate] = useState("bearer");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    try {
      await daemon("/api/connections", {
        method: "POST",
        body: JSON.stringify({
          integration,
          name,
          template,
          values: token ? { token } : {},
        }),
      });
      toast.success("Conexão criada. O valor da chave não será exibido de novo.");
      setToken("");
      onCreated();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não criou a conexão");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Nova conexão</DialogTitle>
          <DialogDescription>
            O segredo é gravado só no cofre. Use o nome da conexão (`work`,
            `pessoal`) — ele faz parte do endereço da ferramenta.
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="c-int">Integração</FieldLabel>
            <Input
              id="c-int"
              value={integration}
              onChange={(event) => setIntegration(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="c-name">Nome</FieldLabel>
            <Input
              id="c-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="c-tpl">Modelo de auth</FieldLabel>
            <Input
              id="c-tpl"
              value={template}
              onChange={(event) => setTemplate(event.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="c-tok">Token / chave (somente escrita)</FieldLabel>
            <Input
              id="c-tok"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(event) => setToken(event.target.value)}
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
