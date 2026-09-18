import type { NextConfig } from "next";

const daemon =
  process.env.EXECUTOR_DAEMON_ORIGIN ?? "http://127.0.0.1:4788";

const nextConfig: NextConfig = {
  agentRules: false,
  async redirects() {
    return [
      { source: "/secrets", destination: "/segredos", permanent: false },
      { source: "/policies", destination: "/politicas", permanent: false },
      { source: "/tools", destination: "/ferramentas", permanent: false },
      {
        source: "/integrations/browse",
        destination: "/integracoes/explorar",
        permanent: false,
      },
      {
        source: "/integrations/add/:plugin",
        destination: "/integracoes/adicionar/:plugin",
        permanent: false,
      },
      {
        source: "/integrations/:slug",
        destination: "/integracoes/:slug",
        permanent: false,
      },
      { source: "/integrations", destination: "/", permanent: false },
      {
        source: "/connect/:slug",
        destination: "/conectar/:slug",
        permanent: false,
      },
      { source: "/enrich-arena", destination: "/arena", permanent: false },
      { source: "/orgs", destination: "/equipes", permanent: false },
      { source: "/artifacts", destination: "/artefatos", permanent: false },
    ];
  },
  async rewrites() {
    return [
      {
        source: "/daemon/:path*",
        destination: `${daemon}/:path*`,
      },
    ];
  },
};

export default nextConfig;
