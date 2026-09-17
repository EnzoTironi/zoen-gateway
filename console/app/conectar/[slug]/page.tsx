"use client";

import { ErrorBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { daemon } from "@/lib/daemon";
import { useParams, useRouter } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";

export default function ConnectPage() {
  const params = useParams<{ slug: string }>();
  const router = useRouter();
  const slug = params.slug;
  const [name, setName] = useState("work");
  const [template, setTemplate] = useState("bearer");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await daemon("/api/connections", {
        method: "POST",
        body: JSON.stringify({
          integration: slug,
          name,
          template,
          values: token ? { token } : {},
        }),
      });
      toast.success(`Conta ${slug}/${name} conectada. A chave não será mostrada de novo.`);
      router.push("/conexoes");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Não conectou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto flex w-full max-w-lg flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">
          Conectar {slug}
        </h1>
        <p className="text-muted-foreground text-sm">
          Cole um token ou chave de API. OAuth com PKCE pausa a execução e abre
          a tela Retomar — não imprimimos só a URL.
        </p>
      </div>
      <form className="flex flex-col gap-4" onSubmit={(event) => void submit(event)}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="name">Nome da conexão</FieldLabel>
            <Input
              id="name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="template">Modelo</FieldLabel>
            <Input
              id="template"
              value={template}
              onChange={(event) => setTemplate(event.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="token">Segredo (somente escrita)</FieldLabel>
            <Input
              id="token"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(event) => setToken(event.target.value)}
            />
          </Field>
        </FieldGroup>
        {error ? <ErrorBlock message={error} /> : null}
        <Button type="submit" disabled={busy}>
          {busy ? <Spinner data-icon="inline-start" /> : null}
          Salvar conexão
        </Button>
      </form>
    </div>
  );
}
