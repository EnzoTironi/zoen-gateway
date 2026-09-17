import type { NextConfig } from "next";

const daemon =
  process.env.EXECUTOR_DAEMON_ORIGIN ?? "http://127.0.0.1:4788";

const nextConfig: NextConfig = {
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
