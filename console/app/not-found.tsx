import Link from "next/link";
import { Button } from "@/components/ui/button";

export default function NotFound() {
  return (
    <div className="flex flex-col gap-3">
      <h1 className="text-2xl font-semibold tracking-tight">Página não encontrada</h1>
      <p className="text-muted-foreground text-sm">
        Esse caminho não existe nesta consola.
      </p>
      <Button className="w-fit" render={<Link href="/" />}>
        Voltar ao catálogo
      </Button>
    </div>
  );
}
