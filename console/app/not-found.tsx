import Link from "next/link";
import { PageHeader } from "@/components/page";
import { Button } from "@/components/ui/button";

export default function NotFound() {
  return (
    <>
      <PageHeader
        title="Página não encontrada"
        description="Esse caminho não existe nesta consola."
      />
      <Button className="w-fit" render={<Link href="/" />}>
        Voltar às integrações
      </Button>
    </>
  );
}
