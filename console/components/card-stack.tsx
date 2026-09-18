"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { SearchIcon, XIcon } from "lucide-react";
import Link from "next/link";
import {
  cloneElement,
  createContext,
  isValidElement,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactElement,
} from "react";

type CardStackContextValue = {
  searchable: boolean;
  searchQuery: string;
  setSearchQuery: (query: string) => void;
};

const CardStackContext = createContext<CardStackContextValue>({
  searchable: false,
  searchQuery: "",
  setSearchQuery: () => {},
});

type CardStackProps = React.ComponentProps<"div"> & {
  searchable?: boolean;
  searchQuery?: string;
  defaultSearchQuery?: string;
  onSearchChange?: (query: string) => void;
};

/**
 * Pilha de linhas com borda fina — o bloco de lista do Executor original.
 * Sem Radix Collapsible: o cabeçalho e as entradas bastam para a consola.
 */
export function CardStack({
  className,
  searchable = false,
  searchQuery: searchQueryProp,
  defaultSearchQuery = "",
  onSearchChange,
  ...props
}: CardStackProps) {
  const [uncontrolledQuery, setUncontrolledQuery] = useState(defaultSearchQuery);
  const searchQuery = searchQueryProp ?? uncontrolledQuery;
  const setSearchQuery = useCallback(
    (query: string) => {
      if (searchQueryProp === undefined) {
        setUncontrolledQuery(query);
      }
      onSearchChange?.(query);
    },
    [onSearchChange, searchQueryProp],
  );
  const value = useMemo(
    () => ({ searchable, searchQuery, setSearchQuery }),
    [searchQuery, searchable, setSearchQuery],
  );

  return (
    <CardStackContext.Provider value={value}>
      <div
        data-slot="card-stack"
        className={cn(
          "bg-card text-card-foreground flex flex-col overflow-hidden rounded-lg border border-border/50 focus-within:!opacity-100",
          className,
        )}
        {...props}
      />
    </CardStackContext.Provider>
  );
}

function CardStackSearchInput() {
  const { searchQuery, setSearchQuery } = useContext(CardStackContext);
  return (
    <div
      data-slot="card-stack-search"
      className="border-input bg-background text-muted-foreground focus-within:border-ring focus-within:ring-ring/40 flex shrink-0 items-center gap-1.5 rounded-md border px-2 py-1 focus-within:ring-1"
    >
      <SearchIcon aria-hidden className="size-3.5 shrink-0" />
      <Input
        type="text"
        value={searchQuery}
        onChange={(event) => setSearchQuery(event.target.value)}
        placeholder="Buscar…"
        aria-label="Buscar entradas"
        className="placeholder:text-muted-foreground h-5 w-32 rounded-none border-0 bg-transparent p-0 text-xs shadow-none outline-none focus-visible:border-0 focus-visible:ring-0 md:text-xs dark:bg-transparent"
      />
      {searchQuery ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Limpar busca"
          onClick={() => setSearchQuery("")}
          className="text-muted-foreground hover:text-foreground size-4 rounded-sm hover:bg-transparent"
        >
          <XIcon aria-hidden className="size-3" />
        </Button>
      ) : null}
    </div>
  );
}

export function CardStackHeader({
  className,
  children,
  rightSlot,
  ...props
}: React.HTMLAttributes<HTMLElement> & { rightSlot?: React.ReactNode }) {
  const { searchable } = useContext(CardStackContext);
  return (
    <div
      data-slot="card-stack-header"
      className={cn(
        "flex w-full items-center justify-between gap-4 px-4 py-3 text-sm font-medium",
        className,
      )}
      {...props}
    >
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {searchable ? <CardStackSearchInput /> : null}
      {rightSlot}
    </div>
  );
}

export function CardStackContent({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-content"
      className={cn(
        "flex flex-col border-t border-border/50 first:border-t-0",
        "[&>*+*]:relative [&>*+*]:before:pointer-events-none [&>*+*]:before:absolute [&>*+*]:before:inset-x-0 [&>*+*]:before:top-0 [&>*+*]:before:h-px [&>*+*]:before:bg-border/50",
        className,
      )}
      {...props}
    />
  );
}

const entryClassName =
  "group/card-stack-entry flex w-full items-center gap-3 px-4 py-3 text-sm outline-none transition-[background-color] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] focus-visible:bg-accent/40 [&[href]]:cursor-pointer [&[href]]:hover:bg-accent/40";

type CardStackEntryProps = React.ComponentProps<"div"> & {
  asChild?: boolean;
  href?: string;
  searchText?: string;
};

export function CardStackEntry({
  className,
  asChild = false,
  href,
  searchText,
  children,
  onClick,
  ...props
}: CardStackEntryProps) {
  const { searchable, searchQuery } = useContext(CardStackContext);
  if (searchable && searchText !== undefined) {
    const trimmed = searchQuery.trim().toLowerCase();
    if (trimmed.length > 0 && !searchText.toLowerCase().includes(trimmed)) {
      return null;
    }
  }
  const classes = cn(entryClassName, className);
  if (asChild && isValidElement(children)) {
    const child = children as ReactElement<{ className?: string }>;
    return cloneElement(child, {
      className: cn(classes, child.props.className),
    });
  }
  if (href) {
    return (
      <Link href={href} className={classes} onClick={onClick as never}>
        {children}
      </Link>
    );
  }
  return (
    <div
      data-slot="card-stack-entry"
      className={classes}
      onClick={onClick}
      {...props}
    >
      {children}
    </div>
  );
}

export function CardStackEntryField({
  className,
  label,
  description,
  hint,
  labelAction,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  label?: React.ReactNode;
  description?: React.ReactNode;
  hint?: React.ReactNode;
  labelAction?: React.ReactNode;
}) {
  return (
    <div
      data-slot="card-stack-entry-field"
      className={cn(
        "flex w-full flex-col items-stretch gap-2 px-4 py-3 text-sm outline-none",
        className,
      )}
      {...props}
    >
      {label || labelAction ? (
        <div className="flex items-center justify-between gap-2">
          {label ? (
            <Label className="text-sm font-medium">
              {label}
              {description ? (
                <span className="text-muted-foreground font-normal">
                  {" "}
                  {description}
                </span>
              ) : null}
            </Label>
          ) : null}
          {labelAction}
        </div>
      ) : null}
      {children}
      {hint ? <p className="text-muted-foreground text-sm">{hint}</p> : null}
    </div>
  );
}

export function CardStackEntryMedia({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-entry-media"
      className={cn(
        "bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-md [&_svg:not([class*='size-'])]:size-4",
        className,
      )}
      {...props}
    />
  );
}

export function CardStackEntryContent({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-entry-content"
      className={cn("flex min-w-0 flex-1 flex-col gap-0.5", className)}
      {...props}
    />
  );
}

export function CardStackEntryTitle({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-entry-title"
      className={cn("truncate text-sm leading-snug font-medium", className)}
      {...props}
    />
  );
}

export function CardStackEntryDescription({
  className,
  ...props
}: React.ComponentProps<"p">) {
  return (
    <p
      data-slot="card-stack-entry-description"
      className={cn("text-muted-foreground truncate text-xs", className)}
      {...props}
    />
  );
}

export function CardStackEntryActions({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-entry-actions"
      className={cn(
        "text-muted-foreground flex shrink-0 items-center gap-2 text-sm",
        className,
      )}
      {...props}
    />
  );
}

export function CardStackEmpty({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-stack-empty"
      className={cn(
        "text-muted-foreground flex w-full items-center justify-between gap-4 px-4 py-3 text-sm",
        className,
      )}
      {...props}
    />
  );
}

