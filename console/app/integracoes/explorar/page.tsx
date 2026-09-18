"use client";

import { ConnectCatalog } from "@/components/connect-catalog";
import { PageHeader } from "@/components/page";

export default function BrowseIntegrationsPage() {
  return (
    <>
      <PageHeader
        title="Explorar integrações"
        description="Superfície Executor: OpenAPI, GraphQL, MCP e presets Google Discovery. Os provedores Treg aparecem quando o catálogo YAML está carregado."
      />
      <ConnectCatalog />
    </>
  );
}
