"use client";

import {
  CardStack,
  CardStackContent,
  CardStackEntry,
  CardStackEntryActions,
  CardStackEntryContent,
  CardStackEntryDescription,
  CardStackEntryField,
  CardStackEntryTitle,
} from "@/components/card-stack";
import { PageHeader } from "@/components/page";
import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { daemon } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type Org = { slug: string; name: string; owner: string };
type Member = { org: string; email: string; role: string };

export default function OrgsPage() {
  const [orgs, setOrgs] = useState<Org[] | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteOrg, setInviteOrg] = useState("local");
  const [joinCode, setJoinCode] = useState("");
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const body = await daemon<{ orgs: Org[] }>("/api/orgs");
      const rows = body.orgs ?? [];
      setOrgs(rows);
      const first = rows[0]?.slug ?? "local";
      const roster = await daemon<{ members: Member[] }>(
        `/api/orgs/${first}/members`,
      );
      setMembers(roster.members ?? []);
      setError(null);
    } catch (err) {
      setOrgs([]);
      setError(err instanceof Error ? err.message : "Falha ao listar equipes");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function create() {
    setBusy(true);
    try {
      await daemon("/api/orgs", {
        method: "POST",
        body: JSON.stringify({ name }),
      });
      toast.success("Equipe criada");
      setName("");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não criou");
    } finally {
      setBusy(false);
    }
  }

  async function invite() {
    setBusy(true);
    try {
      const out = await daemon<{ code: string }>(
        `/api/orgs/${inviteOrg}/invites`,
        {
          method: "POST",
          body: JSON.stringify({ email: inviteEmail, role: "member" }),
        },
      );
      toast.success(`Convite: ${out.code}`);
      setInviteEmail("");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não convidou");
    } finally {
      setBusy(false);
    }
  }

  async function join() {
    setBusy(true);
    try {
      await daemon("/api/orgs/join", {
        method: "POST",
        body: JSON.stringify({ code: joinCode }),
      });
      toast.success("Entrou na equipe");
      setJoinCode("");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não entrou");
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <PageHeader
        title="Equipes"
        description="Até 10 equipes próprias. Papéis: owner, admin, member, viewer."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {!orgs && !error ? <LoadingBlock /> : null}
      {orgs ? (
        <CardStack className="mb-8" searchable>
          <CardStackContent>
            {orgs.map((org) => (
              <CardStackEntry
                key={org.slug}
                searchText={`${org.slug} ${org.name} ${org.owner}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>{org.name}</CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {org.slug} · dono {org.owner}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CardStack className="mb-8">
        <CardStackContent>
          <CardStackEntryField label="Nova equipe">
            <Input
              id="org-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryActions>
              <Button
                type="button"
                onClick={() => void create()}
                disabled={busy || !name}
              >
                {busy ? <Spinner data-icon="inline-start" /> : null}
                Criar
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
          <CardStackEntryField label="Convidar para">
            <Input
              id="invite-org"
              value={inviteOrg}
              onChange={(event) => setInviteOrg(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntryField label="E-mail">
            <Input
              id="invite-email"
              type="email"
              value={inviteEmail}
              onChange={(event) => setInviteEmail(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryActions>
              <Button
                type="button"
                onClick={() => void invite()}
                disabled={busy || !inviteEmail}
              >
                Convidar
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
          <CardStackEntryField label="Código do convite">
            <Input
              id="join-code"
              value={joinCode}
              onChange={(event) => setJoinCode(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryActions>
              <Button
                type="button"
                onClick={() => void join()}
                disabled={busy || !joinCode}
              >
                Entrar
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
        </CardStackContent>
      </CardStack>
      {members.length > 0 ? (
        <CardStack>
          <CardStackContent>
            {members.map((member) => (
              <CardStackEntry key={`${member.org}:${member.email}`}>
                <CardStackEntryContent>
                  <CardStackEntryTitle>{member.email}</CardStackEntryTitle>
                  <CardStackEntryDescription>
                    {member.role} · {member.org}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
    </>
  );
}
