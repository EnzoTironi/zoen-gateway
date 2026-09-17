"use client";

import { ErrorBlock } from "@/components/states";
import { Button } from "@/components/ui/button";
import { useEffect } from "react";

export default function ErrorPage({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <div className="flex flex-col gap-4">
      <ErrorBlock
        title="Algo deu errado"
        message={error.message || "Erro inesperado na consola."}
      />
      <Button type="button" onClick={reset} className="w-fit">
        Tentar de novo
      </Button>
    </div>
  );
}
