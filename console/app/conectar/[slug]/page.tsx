"use client";

import { PageHeader } from "@/components/page";
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
  const slug = params.slug;

  return (
    <>
      <PageHeader
        title={`Conectar ${slug}`}
        description="Cole um token ou inicie OAuth com PKCE. A pausa abre a tela Retomar — não imprimimos só a URL."
      />
      <div className="flex max-w-lg flex-col gap-10">
        <SecretForm slug={slug} />
        <OauthForm slug={slug} />
      </div>
    </>
  );
}

function SecretForm({ slug }: { slug: string }) {
  const router = useRouter();
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
      toast.success(
        `Conta ${slug}/${name} conectada. A chave não será mostrada de novo.`,
      );
      router.push("/conexoes");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Não conectou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={(event) => void submit(event)}>
      <h2 className="text-lg font-medium">Segredo (somente escrita)</h2>
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
          <FieldLabel htmlFor="token">Token / chave</FieldLabel>
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
  );
}

function OauthForm({ slug }: { slug: string }) {
  const [client, setClient] = useState(slug);
  const [name, setName] = useState("oauth");
  const [authorizationUrl, setAuthorizationUrl] = useState("");
  const [tokenUrl, setTokenUrl] = useState("");
  const [clientId, setClientId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await daemon("/api/oauth/clients", {
        method: "POST",
        body: JSON.stringify({
          slug: client,
          authorizationUrl,
          tokenUrl,
          clientId,
          originIntegration: slug,
        }),
      });
      const started = await daemon<{
        authorizationUrl?: string;
        data?: { authorizationUrl?: string };
      }>("/api/oauth/start", {
        method: "POST",
        body: JSON.stringify({
          client,
          name,
          integration: slug,
        }),
      });
      const url =
        started.authorizationUrl ?? started.data?.authorizationUrl ?? null;
      setAuthUrl(url);
      toast.success("Cliente OAuth gravado. Abra a URL de autorização.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "OAuth não iniciou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={(event) => void submit(event)}>
      <h2 className="text-lg font-medium">OAuth (PKCE)</h2>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="oclient">Slug do cliente</FieldLabel>
          <Input
            id="oclient"
            value={client}
            onChange={(event) => setClient(event.target.value)}
            required
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="oname">Nome da conexão</FieldLabel>
          <Input
            id="oname"
            value={name}
            onChange={(event) => setName(event.target.value)}
            required
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="authz">URL de autorização</FieldLabel>
          <Input
            id="authz"
            value={authorizationUrl}
            onChange={(event) => setAuthorizationUrl(event.target.value)}
            required
            placeholder="https://…/authorize"
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="tokurl">URL de token</FieldLabel>
          <Input
            id="tokurl"
            value={tokenUrl}
            onChange={(event) => setTokenUrl(event.target.value)}
            required
            placeholder="https://…/token"
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="cid">Client ID (público)</FieldLabel>
          <Input
            id="cid"
            value={clientId}
            onChange={(event) => setClientId(event.target.value)}
            required
          />
        </Field>
      </FieldGroup>
      {error ? <ErrorBlock message={error} /> : null}
      {authUrl ? (
        <p className="text-sm">
          Abra{" "}
          <a className="underline underline-offset-4" href={authUrl}>
            a autorização
          </a>{" "}
          e volte em Retomar se a execução pausar.
        </p>
      ) : null}
      <Button type="submit" variant="outline" disabled={busy}>
        {busy ? <Spinner data-icon="inline-start" /> : null}
        Registrar cliente e iniciar
      </Button>
    </form>
  );
}
