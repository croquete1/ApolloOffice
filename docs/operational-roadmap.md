# ApolloOffice — Roadmap operacional (24/11/2026)

## Requisitos do proprietário
- Preservar integralmente a interface atual dos agentes, incluindo canvas pixelizado, personagens, disposição, cores, animações e interações existentes, até autorização expressa para a alterar. Não substituir por grelha, novo cenário ou outra apresentação. Melhorar apenas os restantes módulos operacionais sem interferir com essa interface.
- Qwen não está em utilização. Não selecionar, iniciar, instalar ou exigir Qwen/Ollama no plano de trabalho. Manter compatibilidade opcional existente; apenas usar modelos e fornecedores explicitamente configurados pelo proprietário.
- Implementar todas as melhorias da auditoria incrementalmente, preservando autorização, revisão independente, provas RED/GREEN, CI, atualização assinada e configurações.
- As issues estão desativadas no fork (GitHub HTTP 410); este ficheiro constitui o backlog versionado. Não declarar uma fase concluída sem evidência.

## Estado em 24/11/2026
Base main: 878ef1a45b82319bb912ae567a6cc4956c84ffe4. Release publicada: fork-v0.10.15.
- PR #47 — encerrada sem merge a pedido do proprietário; a proposta de substituir/desativar a interface dos agentes fica retirada.
- [ ] PR #48 — cursor de comentários e confirmação transacional; validar falhas de Guard/Watcher e pedidos concorrentes.
- [ ] PR #49 — melhorias upstream de Docker; validar ciclo real do worker.
PRs #48 e #49 em draft, não integradas nem validadas. O presente roadmap não representa implementação.

## Fases e critérios de aceitação
### F01 / P0 — Integração seletiva upstream
- [ ] Validar #49: init, limites CPU daemon, stderr e contentores pausados; testes de retoma, volumes e cache npm. Sem recriar contentores com jobs ativos.
- [ ] Portar fnm (upstream c464574) mantendo Node 24 do sistema por omissão; versão por repo sem alterar default partilhado. Testar repo Node 20 seguido de repo sem versão, versão inválida e configuração de setup.
- [ ] Adaptar filtros/pesquisa e wrap dos logs do upstream à interface operacional.
- [ ] Preservar canal Ed25519 exclusivo e não importar tags/releases upstream.

### F02 / P0 — Pedidos de revisão duráveis
- [ ] Validar #48; introduzir inbox persistente com recebido/em análise/concluído/retry e next_attempt_at.
- [ ] Não perder pedidos quando um job está ativo ou um fornecedor falha. Deduplicar por repo/PR/HEAD/review-id, conservar evidência e recuperar após restart. Não confirmar uma ação que falhou.
- [ ] Teste end-to-end de falha após leitura antes de Guard, após análise, após enqueue e durante confirmação; não publicar resposta duplicada.

### F03 / P0 — Um PR Steward
- [ ] Centralizar revisão, rebase, recheck e merge no Steward; Watcher deteta acontecimentos e notifica.
- [ ] Migração preserva autorizações existentes e nunca ativa permissões novas. Uma admissão atómica por PR/HEAD; validações finais HEAD/base/tree/CI/revisões, branch protection e threads.
- [ ] Testes de corrida Watcher/Steward, alteração de SHA e falha de GitHub. Sem aprovação automática indevida.

### F04 / P0 — SQLite
- [ ] Result explícito em operações críticas, sem confundir erro com ausência de registos; migrações versionadas e transacionais.
- [ ] Atomicidade, idempotência e constraints de admissão, transições, quotas, aprovação e merge.
- [ ] Testar SQLITE_BUSY, rollback, disco cheio, reinício e falhar fechado. Apresentar diagnóstico.

### F05 / P1 — Retoma diferida e lanes
- [ ] Substituir esperas de rede/limites/CI por estados persistentes, checkpoint e next_attempt_at, libertando lane sem perder a árvore. Backoff limitado, wake por eventos, cancelamento.
- [ ] Não duplicar jobs, perder aprovação ou publicar duas vezes. Testar reinício, concorrência, deadline e fallback; apresentar razões reais de espera.

### F06 / P1 — Independência dos fornecedores
- [ ] Estado, circuit breaker e tempo de recuperação por fornecedor, em modo normal e free-first. Preservar pausas manuais, horários, limites de tokens e orçamento global/repo.
- [ ] Continuar trabalho elegível com outro fornecedor autorizado; sem fallback pago ou Qwen não autorizado. Testar Claude indisponível com Codex elegível e inverso; ausência de rotas; configurações antigas.

### F07 / P1 — Quota Codex preventiva
- [ ] Integrar telemetria existente de 5h/semanal e previsão no scheduler. Reserva configurável para P0/P1 e revisão; adiar hunts/otimizações. Nunca apresentar dados stale como atuais.
- [ ] Reservas concorrentes sem extrapolação fictícia; indicar razão do encaminhamento e retoma. Testar reset, dados desconhecidos e limites manuais.

### F08 / P1 — Routing adaptativo
- [ ] Seleção entre modelos configurados (ex.: Codex/Claude), por papel, complexidade, eficácia medida, custo e disponibilidade. Sem Qwen por omissão ou fallback implícito.
- [ ] Preservar revisão independente e limites; decisões explicáveis e opção de política fixa. Testar falhas, modelos não autorizados, ausência de histórico e revisão.

### F09 / P1 — Sincronização integral
- [ ] Substituir limites 500 issues / 1000 PRs por paginação real e sincronização incremental; reconciliar periodicamente com snapshot completo atómico. Paginar comentários e reviews.
- [ ] Não marcar fechados nem executar ações com dados incompletos. Testar 501 issues, 1001 PRs, falha de página, alterações durante scan, rate limits.

### F10 / P1 — Fila inteligente
- [ ] Dependências explícitas e prevenção de ciclos; desbloquear após merge validado. Prioridades manuais persistentes, fairness por repo, aging.
- [ ] Identificar causa comum e sobreposições antes de executar; sugerir agrupamento sem fechar issues automaticamente. Planeamento de alterações paralelas, revalidar base.
- [ ] Testar grafo, concorrência, quotas, prioridades e ausência de starvation.

### F11 / P1 — Recursos adaptativos
- [ ] Limites de CPU, memória, lanes/CI, prioridade de processos e temperatura respeitando tetos definidos. Métricas de GPU/VRAM apenas quando disponíveis, sem dependência de Ollama.
- [ ] Ajustar novas admissões sem terminar trabalho existente; testes de pressão de recursos e telemetria ausente.

### F12 / P1 — Isolamento
- [ ] Separar workers por confiança; bloquear acesso cruzado a checkouts, segredos e caches executáveis, preservar caches de downloads seguras.
- [ ] Testar Docker real, SSH, credenciais, manutenção e migração sem perda de dados.

### F13 / P2 — Aprendizagem supervisionada
- [ ] Evoluir lições pendentes existentes com aprovação explícita, proveniência, SHA, evidência, escopo e expiração. Nunca promover dados não confiáveis automaticamente.
- [ ] Medir efeito sobre regressões e rejeições sem enfraquecer gates.

### F14 / P2 — Perfis operacionais
- [ ] Económico, equilibrado, desempenho, noturno; limites explícitos, sem ativar Qwen, sem ultrapassar orçamento. Testar transições e reinício.

### F15 / P2 — Dashboard operacional
- [ ] Manter a interface atual dos agentes visual e funcionalmente inalterada. Validar regressão desktop/mobile, canvas, disposição, personagens, animações, navegação e ações; não integrar a PR #47.
- [ ] Apresentar próxima tarefa elegível, dependências, bloqueios, quota, cooldown, razões de seleção, histórico e tempos por fase, com dados reais ou desconhecidos.
- [ ] Não redesenhar, remover, ocultar ou substituir a interface existente dos agentes. Não adicionar novas personagens, cenários ou animações sem autorização expressa.

### F16 / P2 — Métricas e validação
- [ ] Produtividade por repo/modelo, primeira passagem, tempo por fase, taxa de merge e custo por correção validada; distinguir contagens e amostras. Comparar baseline e resultados medidos.
- [ ] Executar cargo fmt --all -- --check, cargo clippy --locked --all-targets -- -D warnings, cargo test --locked, node --test, Python quality/adapters, Chromium desktop/mobile, Docker real e fornecedores autorizados.
- [ ] Não publicar versão assinada sem gates, backup, drain, validação de instalação e rollback. Não declarar conclusão sem resultados.

## Sequência
Integrar PRs apenas após revisão e testes, uma fase de cada vez. Não ativar Qwen. Não alterar main ou instalação em produção sem evidência. Atualizar este ficheiro com PR, SHA e resultado dos testes de cada fase.
