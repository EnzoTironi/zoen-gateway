"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";

const NAV = [
  { href: "/", label: "Catálogo" },
  { href: "/conexoes", label: "Conexões" },
  { href: "/ferramentas", label: "Ferramentas" },
  { href: "/integracoes", label: "Integrações" },
  { href: "/saldo", label: "Saldo" },
] as const;

/**
 * Barra de navegação da consola (pt-BR).
 */
export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  return (
    <div className="flex min-h-full flex-col">
      <header className="border-b bg-background">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex flex-col gap-0.5">
            <Link href="/" className="text-sm font-semibold tracking-tight">
              Executor ∪ Treg
            </Link>
            <p className="text-muted-foreground text-xs">
              Integrações, conexões e catálogo com preço — um token.
            </p>
          </div>
          <nav
            aria-label="Principal"
            className="flex flex-wrap items-center gap-1"
          >
            {NAV.map((item) => {
              const active =
                item.href === "/"
                  ? pathname === "/"
                  : pathname.startsWith(item.href);
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "rounded-md px-2.5 py-1.5 text-sm",
                    active
                      ? "bg-muted font-medium text-foreground"
                      : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                  )}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>
        </div>
      </header>
      <main className="mx-auto flex w-full max-w-6xl flex-1 flex-col gap-6 px-4 py-6">
        {children}
      </main>
    </div>
  );
}
