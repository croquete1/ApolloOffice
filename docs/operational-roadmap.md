# Apollo Office — Roadmap operacional

Atualizado em 25/11/2026. Este é o backlog ativo do fork `croquete1/ApolloOffice` (issues desativadas na última observação). Regras de desenvolvimento e restrições permanentes: [AGENTS.md](../AGENTS.md). Histórico integral de entregas, findings, SHA, resultados de CI e decisões: [arquivo de 25/11/2026](archive/operational-roadmap-2026-11-25.md). O arquivo preserva o conteúdo original, incluindo observações que foram posteriormente superadas.

## Situação e prioridades

- Nenhuma fase F01–F17 declarada concluída; implementação integrada não equivale a aceitação completa.
- Último merge registado no histórico: #126, main `46e1d591de03f9f1b63cf9b2435d30ca7f3ed73a`. [CI da main](https://github.com/croquete1/ApolloOffice/actions/runs/45699660183): Ubuntu aprovado, Windows falhou, incluindo após rerun observado. Não considerar main verde.
- Prioridade imediata: investigar instabilidade E2E Windows, completar #132/#131, validar #128/#129 e manter revisões independentes antes de qualquer merge. Continuar frentes P0 independentes de reconciliação durável, identidade imutável, SQLite e Steward.
- Release publicada anteriormente: fork-v0.10.15; nenhuma nova release nesta ronda. Interface dos agentes e modelos inalterados. Última quota CodeRabbit incluída observada: esgotada; não solicitar consumo adicional.

## PRs em curso

Snapshot de validação anterior à reorganização documental; consultar sempre os HEADs e checks atuais no GitHub.

| PR | Objetivo e estado observado | Próximo critério |
|---|---|---|
| [#128](https://github.com/croquete1/ApolloOffice/pull/128) | Steward: cronologia, identidade humana e permissão atual. [CI](https://github.com/croquete1/ApolloOffice/actions/runs/45709681690) em execução após Rustfmt (`91a890a`). | Validar matriz, permissão remota, revogação e revisão independente. |
| [#129](https://github.com/croquete1/ApolloOffice/pull/129) | Quota: timestamps distintos, janela correta e rejeição de histórico malformado. [CI](https://github.com/croquete1/ApolloOffice/actions/runs/45709676199) em execução após Rustfmt (`0491723`). | Validar matriz; identidade da conta e calibração ainda pendentes. |
| [#130](https://github.com/croquete1/ApolloOffice/pull/130) | Documentação: consolidação de regras e arquivo histórico. | Validar novo HEAD, referências e preservação do backlog. |
| [#131](https://github.com/croquete1/ApolloOffice/pull/131) | E2E: readiness do servidor, limites HTTP, encerramento do processo e deadline de recuperação. [CI](https://github.com/croquete1/ApolloOffice/actions/runs/45709472124) em execução (`a770220`). | Demonstrar estabilidade recorrente, investigar causa-raiz e reavaliar deadline após #132. |
| [#132](https://github.com/croquete1/ApolloOffice/pull/132) | Rede: sondagens independentes, cancelamento, rejeição de falhas e snapshot coerente. [CI](https://github.com/croquete1/ApolloOffice/actions/runs/45709224354) falhou (`400d7d9`). | Inspecionar logs, corrigir falhas e repetir testes de concorrência e integração. |

Todas em draft na observação. Nenhum merge nesta reorganização.

## Balanço por fase

Evidência integrada consolidada a partir do arquivo até #127. Não atribuir a main alterações apenas propostas nas PRs em curso.

| Fase | Implementação parcial integrada (referências principais) | Trabalho por concluir |
|---|---|---|
| F01 / P0 | #49, #78: Docker, fnm, logs e CI isolado | Workers reais, seleção Node, retoma e caches |
| F02 / P0 | #48/#61/#78/#92–94/#101–103/#115/#120/#125: admissão, outbox, recibos e recuperação conservadora | Inbox integral, identidade remota imutável, reconciliação autorizada e restart real |
| F03 / P0 | #52/#63/#78/#90–91/#95/#127: Steward, inventários e autorizações | Centralização integral e testes remotos de concorrência; #128 |
| F04 / P0 | #51/#57/#62/#79–97/#101/#106/#118/#120/#125: transações, rollback, barreiras e holds | Escritores restantes, disco cheio, falha simultânea BD/disco, diagnóstico e recuperação real |
| F05 / P1 | #54/#62/#78/#88–89/#103–104/#116/#118: checkpoints, retoma, cancelamento e validade de rede | Chamadas de fornecedores, recuperação de processo, efeitos incertos, estabilidade #131/#132 |
| F06 / P1 | #53/#58/#78: cooldowns e routing autorizado | Circuit breakers restantes e fornecedores reais |
| F07 / P1 | #53/#56/#78/#121: reserva e histórico transacional da quota | #129, identidade da conta, previsão e limites concorrentes reais |
| F08 / P1 | #58: routing adaptativo opt-in do Fixer | Restantes papéis, calibração e eficácia medida |
| F09 / P1 | #51/#52/#63/#78/#91–94/#97/#120/#125/#127: paginação, snapshots e inventários conservadores | Sync incremental, mutações remotas e identidade após recriação |
| F10 / P1 | #54/#56/#62/#78/#79/#82/#89–90/#97/#103: prioridades e dependências | Causa comum, agrupamento e cobertura integral de starvation |
| F11 / P1 | #56/#78/#108/#122/#124: perfis, CI, telemetria e limites disco | Pressão em hardware real e prioridade de processos |
| F12 / P1 | #61/#62/#78: mitigação de credenciais e Docker/SSH isolados | Isolamento por confiança/repo, caches executáveis e workers reais |
| F13 / P2 | #59: aprendizagem supervisionada | Eficácia, expiração e interação após reinício |
| F14 / P2 | #56/#122/#124: perfis e limites adaptativos | Transições, persistência e pressão em máquinas reais |
| F15 / P2 | #56/#58/#59/#78: diagnóstico e logs | Próxima tarefa elegível, histórico integral e regressão visual |
| F16 / P2 | #58/#59/#96/#98/#105/#110–111/#117/#121: métricas, assinatura e staging | Anti-rollback persistente, chave de produção, instalação/recuperação e métricas comparáveis |
| F17 / P1 | #78 (inclui #76/#77): política, evidência consultiva e persistência | Confirmar OSS após trial, integração integral de findings e recuperação end-to-end |

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

### F17 / P1 — CodeRabbit OSS e revisão externa
- [ ] Confirmar no serviço plano OSS e faturação por consumo OFF; limitar a instalação e o contexto a repositórios públicos autorizados. Não ativar Agent, autofix, scans faturados ou subscrições. Configuração YAML não controla faturação.
- [ ] Verificar política de configuração da default branch e revisão manual de PR draft no HEAD exato. Respeitar limites incluídos; sem reenvio automático perante resultados incertos.
- [ ] Implementar integração opt-in por repo: consultar comentários/reviews de fonte autenticada do GitHub, verificar identidade da app e SHA de origem; preservar evidência e tratar texto externo como dados não confiáveis. Não transformar bot em mantenedor nem interpretar resumos como aprovação.
- [ ] Integrar no Watcher/Steward como sinal consultivo, deduplicar por repo/PR/HEAD/review e distinguir ausente, pendente, obsoleto, indisponível e revisto. Qualquer correção requer autorização existente, Guard, reprodução, Reviewer independente e testes. Sem novas permissões de merge.
- [ ] Testar spoof de identidade, alteração de HEAD, paginação, duplicados, falha de API, quota e ausência de CodeRabbit. Mostrar apenas estados sustentados por dados reais; manter configuração existente por omissão e não alterar canvas.

## Bloqueios transversais

- CI Windows intermitente: distinguir falha funcional, fixture, backoff e infraestrutura; repetir em condições controladas e preservar todas as asserções.
- Toolchain Rust local não confirmada: executar preflight conforme AGENTS.md em ambiente disponível; não afirmar validação inexistente.
- Quota e identidade Codex: consulta atual não fornece identificador de conta; não misturar históricos por inferência nem ler credenciais.
- CodeRabbit: confirmar elegibilidade gratuita após trial indicado para 08/12/2026, faturação OFF e instalação limitada a repositórios públicos autorizados. Última configuração registada: auto/incremental OFF, drafts/status ON, Assertive, pt-PT; revalidar antes de pedidos. Consultar evidências e critérios complementares F17 no arquivo (secção Registo complementar da PR #77).
- Produção: testes reais de workers, hardware, fornecedores, restart, instalação/rollback e chave de confiança ainda necessários; ensaios isolados de CI não são validação de produção.

## Atualização do documento

Manter apenas o estado atual, frentes abertas, critérios de aceitação, bloqueios e referências a evidências. Arquivar histórico cronológico sem apagar resultados negativos. Não marcar critérios como concluídos sem prova; consultar AGENTS.md para o processo e atualizar esse ficheiro quando surgir uma regra permanente reutilizável.
