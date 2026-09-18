"use client";

import { CommandPalette } from "@/components/command-palette";
import { PageContainer } from "@/components/page";
import { Wordmark } from "@/components/wordmark";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { daemon } from "@/lib/daemon";
import { cn } from "@/lib/utils";
import { BookOpenIcon, CommandIcon, PlusIcon } from "lucide-react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";

const PRIMARY_NAV = [
  { href: "/", label: "Integrações" },
  { href: "/segredos", label: "Provedores" },
  { href: "/politicas", label: "Políticas" },
  { href: "/toolkits", label: "Toolkits" },
  { href: "/artefatos", label: "Artefatos" },
] as const;

const TREG_NAV = [
  { href: "/catalogo", label: "Catálogo" },
  { href: "/saldo", label: "Saldo" },
  { href: "/arena", label: "Arena" },
  { href: "/equipes", label: "Equipes" },
  { href: "/skills", label: "Skills" },
  { href: "/ferramentas", label: "Ferramentas" },
] as const;

type Integration = { slug: string; name: string; kind: string };

/**
 * Shell do Executor: sidebar, ⌘K, lista de integrações e extras Treg.
 * O scroll e a coluna `max-w-4xl` ficam no PageContainer, como no original.
 */
export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [commandsOpen, setCommandsOpen] = useState(false);

  useEffect(() => {
    setMobileOpen(false);
  }, [pathname]);

  useEffect(() => {
    if (!mobileOpen) {
      return;
    }
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, [mobileOpen]);

  return (
    <div className="flex h-svh overflow-hidden">
      <CommandPalette open={commandsOpen} onOpenChange={setCommandsOpen} />
      <aside className="bg-sidebar border-sidebar-border hidden w-52 shrink-0 flex-col border-r md:flex lg:w-56">
        <SidebarContent
          pathname={pathname}
          onOpenCommands={() => setCommandsOpen(true)}
        />
      </aside>
      {mobileOpen ? (
        <div className="fixed inset-0 z-50 flex md:hidden">
          <button
            type="button"
            aria-label="Fechar navegação"
            className="absolute inset-0 bg-black/45 backdrop-blur-[1px]"
            onClick={() => setMobileOpen(false)}
          />
          <div className="bg-sidebar border-sidebar-border relative flex h-full w-[84vw] max-w-xs flex-col border-r shadow-2xl">
            <div className="border-sidebar-border flex h-12 shrink-0 items-center justify-between border-b px-4">
              <Wordmark onNavigate={() => setMobileOpen(false)} />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label="Fechar navegação"
                onClick={() => setMobileOpen(false)}
              >
                <svg viewBox="0 0 16 16" className="size-3.5">
                  <path
                    d="M3 3l10 10M13 3L3 13"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                  />
                </svg>
              </Button>
            </div>
            <SidebarContent
              pathname={pathname}
              onNavigate={() => setMobileOpen(false)}
              showBrand={false}
              onOpenCommands={() => {
                setMobileOpen(false);
                setCommandsOpen(true);
              }}
            />
          </div>
        </div>
      ) : null}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="bg-background flex h-12 shrink-0 items-center justify-between border-b px-4 md:hidden">
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Abrir navegação"
            onClick={() => setMobileOpen(true)}
          >
            <svg viewBox="0 0 16 16" className="size-4">
              <path
                d="M2 4h12M2 8h12M2 12h12"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
              />
            </svg>
          </Button>
          <Wordmark />
          <div className="w-8 shrink-0" />
        </div>
        <PageContainer>{children}</PageContainer>
      </main>
    </div>
  );
}

function SidebarContent({
  pathname,
  onNavigate,
  showBrand = true,
  onOpenCommands,
}: {
  pathname: string;
  onNavigate?: () => void;
  showBrand?: boolean;
  onOpenCommands: () => void;
}) {
  return (
    <>
      {showBrand ? (
        <div className="border-sidebar-border flex h-12 shrink-0 items-center border-b px-4">
          <Wordmark onNavigate={onNavigate} />
        </div>
      ) : null}
      <nav className="flex flex-1 flex-col overflow-y-auto p-2" aria-label="Principal">
        {PRIMARY_NAV.map((item) => (
          <NavItem
            key={item.href}
            item={item}
            pathname={pathname}
            onNavigate={onNavigate}
          />
        ))}
        <p className="text-muted-foreground mt-5 mb-1 px-2.5 text-xs font-medium tracking-widest uppercase">
          Treg
        </p>
        {TREG_NAV.map((item) => (
          <NavItem
            key={item.href}
            item={item}
            pathname={pathname}
            onNavigate={onNavigate}
          />
        ))}
        <div className="text-muted-foreground mt-5 mb-1 flex items-center justify-between px-2.5 text-xs font-medium tracking-widest uppercase">
          <span>Integrações</span>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            nativeButton={false}
            aria-label="Explorar integrações"
            render={<Link href="/integracoes/explorar" onClick={onNavigate} />}
          >
            <PlusIcon />
          </Button>
        </div>
        <IntegrationList pathname={pathname} onNavigate={onNavigate} />
      </nav>
      <div className="border-sidebar-border shrink-0 border-t p-2">
        <button
          type="button"
          onClick={onOpenCommands}
          className="text-sidebar-foreground hover:bg-sidebar-active/60 hover:text-foreground group flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors"
        >
          <CommandIcon className="size-4 shrink-0" />
          <span className="flex-1 text-left">Comandos</span>
          <span className="text-muted-foreground font-mono text-[11px]">⌘K</span>
        </button>
        <a
          href="https://executor.sh/docs"
          target="_blank"
          rel="noopener noreferrer"
          className="text-sidebar-foreground hover:bg-sidebar-active/60 hover:text-foreground group flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm"
        >
          <BookOpenIcon className="size-4 shrink-0" />
          <span className="flex-1">Docs</span>
        </a>
      </div>
    </>
  );
}

function NavItem({
  item,
  pathname,
  onNavigate,
}: {
  item: { href: string; label: string };
  pathname: string;
  onNavigate?: () => void;
}) {
  const active =
    item.href === "/"
      ? pathname === "/"
      : pathname === item.href || pathname.startsWith(`${item.href}/`);
  return (
    <Link
      href={item.href}
      onClick={onNavigate}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors",
        active
          ? "bg-sidebar-active text-foreground font-medium"
          : "text-sidebar-foreground hover:bg-sidebar-active/60 hover:text-foreground",
      )}
    >
      {item.label}
    </Link>
  );
}

function IntegrationList({
  pathname,
  onNavigate,
}: {
  pathname: string;
  onNavigate?: () => void;
}) {
  const [rows, setRows] = useState<Integration[] | null>(null);

  useEffect(() => {
    daemon<{ integrations: Integration[] }>("/api/integrations")
      .then((body) => setRows(body.integrations))
      .catch(() => setRows([]));
  }, []);

  if (rows === null) {
    return (
      <div className="flex flex-col gap-1 px-2.5 py-1">
        {[80, 65, 72, 58, 68].map((width) => (
          <div key={width} className="flex items-center gap-2 rounded-md py-1.5">
            <Skeleton className="size-3.5 shrink-0 rounded" />
            <Skeleton className="h-3" style={{ width: `${width}%` }} />
          </div>
        ))}
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <p className="text-muted-foreground px-2.5 py-2 text-sm leading-relaxed">
        Nenhuma integração ainda
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-px">
      {rows.map((row) => {
        const href = `/integracoes/${row.slug}`;
        const active = pathname === href || pathname.startsWith(`${href}/`);
        return (
          <Link
            key={row.slug}
            href={href}
            onClick={onNavigate}
            className={cn(
              "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-colors",
              active
                ? "bg-sidebar-active text-foreground font-medium"
                : "text-sidebar-foreground hover:bg-sidebar-active/60 hover:text-foreground",
            )}
          >
            <span className="bg-muted text-muted-foreground flex size-3.5 shrink-0 items-center justify-center rounded text-[9px] font-medium">
              {(row.name || row.slug).slice(0, 1).toUpperCase()}
            </span>
            <span className="flex-1 truncate">{row.name || row.slug}</span>
          </Link>
        );
      })}
    </div>
  );
}
