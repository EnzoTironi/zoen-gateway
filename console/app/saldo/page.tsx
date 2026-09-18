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
import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { daemon, formatMicroUsd } from "@/lib/daemon";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

type Balance = {
  subject: string;
  balance_micro: number;
  currency: string;
  topup_url: string;
};

type Topup = {
  mode: "local" | "stripe";
  id: string;
  amount_micro: number;
  checkout_url?: string;
  confirm_url?: string;
};

export default function BalancePage() {
  const [data, setData] = useState<Balance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"grant" | "topup" | null>(null);

  const load = useCallback(async () => {
    try {
      setData(await daemon<Balance>("/api/balance"));
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Falha ao ler o saldo");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function grant() {
    setBusy("grant");
    try {
      await daemon("/api/balance/grant", {
        method: "POST",
        body: JSON.stringify({ micro: 1_000_000 }),
      });
      toast.success("Crédito local de US$ 1,00 adicionado");
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não creditou");
    } finally {
      setBusy(null);
    }
  }

  async function checkout() {
    setBusy("topup");
    try {
      const intent = await daemon<Topup>("/api/balance/topup", {
        method: "POST",
        body: JSON.stringify({ micro: 5_000_000 }),
      });
      if (intent.mode === "stripe" && intent.checkout_url) {
        window.location.href = intent.checkout_url;
        return;
      }
      if (intent.confirm_url) {
        await daemon(intent.confirm_url, { method: "POST", body: "{}" });
        toast.success("Top-up local de US$ 5,00 confirmado");
        await load();
      }
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Não abriu o checkout");
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      <PageHeader
        title="Saldo"
        description="Ledger em micro-USD. Sem STRIPE_SECRET_KEY o checkout confirma localmente. A sua chave nunca é cobrada."
      />
      {error ? <ErrorBlock message={error} /> : null}
      {!data && !error ? <LoadingBlock /> : null}
      {data ? (
        <CardStack>
          <CardStackContent>
            <CardStackEntry>
              <CardStackEntryContent>
                <CardStackEntryTitle className="font-display text-[1.75rem] tracking-tight">
                  {formatMicroUsd(data.balance_micro)}
                </CardStackEntryTitle>
                <CardStackEntryDescription>
                  Sujeito {data.subject} · 402 inclui topup_url
                </CardStackEntryDescription>
              </CardStackEntryContent>
              <CardStackEntryActions>
                <Button
                  type="button"
                  onClick={() => void checkout()}
                  disabled={busy !== null}
                >
                  {busy === "topup" ? (
                    <Spinner data-icon="inline-start" />
                  ) : null}
                  Top-up US$ 5,00
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void grant()}
                  disabled={busy !== null}
                >
                  {busy === "grant" ? (
                    <Spinner data-icon="inline-start" />
                  ) : null}
                  Recarregar US$ 1,00
                </Button>
              </CardStackEntryActions>
            </CardStackEntry>
          </CardStackContent>
        </CardStack>
      ) : null}
    </>
  );
}
