# Consola Executor ∪ Treg

Interface em **português do Brasil**: Catálogo (busca por trabalho), Conexões, Ferramentas, Integrações, Saldo e Retomar.

```bash
# terminal 1 — daemon
executor daemon run --foreground --port 4788

# terminal 2 — consola
npm install
npm run dev
```

Abre http://127.0.0.1:43123. O Next faz rewrite de `/daemon/*` para o daemon (`EXECUTOR_DAEMON_ORIGIN`, padrão `http://127.0.0.1:4788`). `GET /api/console/bootstrap` entrega o token local.
