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
import { EmptyBlock, ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { daemon } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type Skill = { slug: string; name: string; body: string };

export default function SkillsPage() {
  const [rows, setRows] = useState<Skill[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [slug, setSlug] = useState("");
  const [body, setBody] = useState("# Minha skill\n\nReceita compartilhada.");

  const load = useCallback(async () => {
    try {
      const out = await daemon<{ skills: Skill[] }>("/api/skills");
      setRows(out.skills ?? []);
      setError(null);
    } catch (err) {
      setRows([]);
      setError(err instanceof Error ? err.message : "Falha ao listar skills");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function add() {
    try {
      await daemon("/api/skills", {
        method: "POST",
        body: JSON.stringify({ slug, name: slug, body }),
      });
      toast.success("Skill registrada");
      setSlug("");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não registrou");
    }
  }

  return (
    <>
      <PageHeader
        title="Skills"
        description="Pacote SKILL.md compartilhado. O MCP skills lista estes slugs junto dos guias nativos."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {!rows && !error ? <LoadingBlock /> : null}
      {rows && rows.length === 0 && !error ? (
        <EmptyBlock
          title="Nenhuma skill"
          description="Envie um SKILL.md ou use executor upload skills --dir."
        />
      ) : null}
      {rows && rows.length > 0 ? (
        <CardStack className="mb-8" searchable>
          <CardStackContent>
            {rows.map((row) => (
              <CardStackEntry
                key={row.slug}
                searchText={`${row.slug} ${row.name} ${row.body}`}
              >
                <CardStackEntryContent>
                  <CardStackEntryTitle>
                    {row.name || row.slug}
                  </CardStackEntryTitle>
                  <CardStackEntryDescription className="whitespace-pre-wrap">
                    {row.body.slice(0, 160)}
                  </CardStackEntryDescription>
                </CardStackEntryContent>
              </CardStackEntry>
            ))}
          </CardStackContent>
        </CardStack>
      ) : null}
      <CardStack>
        <CardStackContent>
          <CardStackEntryField label="Slug">
            <Input
              id="skill-slug"
              value={slug}
              onChange={(event) => setSlug(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntryField label="SKILL.md">
            <Textarea
              id="skill-body"
              rows={8}
              value={body}
              onChange={(event) => setBody(event.target.value)}
            />
          </CardStackEntryField>
          <CardStackEntry>
            <CardStackEntryActions>
              <Button type="button" onClick={() => void add()} disabled={!slug}>
                Registrar
              </Button>
            </CardStackEntryActions>
          </CardStackEntry>
        </CardStackContent>
      </CardStack>
    </>
  );
}
