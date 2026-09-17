"use client";

import { ThemeProvider } from "next-themes";
import { Toaster } from "@/components/ui/sonner";

/**
 * Tema + toasts da consola.
 */
export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
      {children}
      <Toaster richColors closeButton position="top-right" />
    </ThemeProvider>
  );
}
