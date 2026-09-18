# Consola Executor ∪ Treg

Interface em **português do Brasil**, no desenho do Executor original: sidebar (Integrações, Provedores, Políticas, Toolkits), card **Conectar um agente** na home, extras Treg (Catálogo, Saldo, Arena, Equipes, Skills, Artefatos). O daemon também serve a SPA em `http://127.0.0.1:4788` (`executor web`).

```bash
# terminal 1 — daemon
executor daemon run --foreground --port 4788

# terminal 2 — consola
npm install
npm run dev
```

Abre http://127.0.0.1:43123. O Next faz rewrite de `/daemon/*` para o daemon (`EXECUTOR_DAEMON_ORIGIN`, padrão `http://127.0.0.1:4788`). `GET /api/console/bootstrap` entrega o token local.
