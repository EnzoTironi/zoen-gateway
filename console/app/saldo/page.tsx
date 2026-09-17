"use client";

import { ErrorBlock, LoadingBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
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

export default function BalancePage() {
  const [data, setData] = useState<Balance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
    setBusy(true);
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
      setBusy(false);
    }
  }

  if (error) {
    return <ErrorBlock message={error} />;
  }
  if (!data) {
    return <LoadingBlock />;
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <h1 className="text-2xl font-semibold tracking-tight">Saldo</h1>
        <p className="text-muted-foreground text-sm">
          Ledger local em micro-USD. Stripe fica para o próximo corte. A sua
          chave nunca é cobrada.
        </p>
      </div>
      <Card className="max-w-md">
        <CardHeader>
          <CardTitle>{formatMicroUsd(data.balance_micro)}</CardTitle>
          <CardDescription>
            Sujeito {data.subject} · crédito de cadastro aplicado uma vez por
            processo.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button type="button" onClick={() => void grant()} disabled={busy}>
            {busy ? <Spinner data-icon="inline-start" /> : null}
            Recarregar US$ 1,00 (local)
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
