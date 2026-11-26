# Apollo Office — Roadmap operacional

Atualizado em 26/11/2026. Este é o backlog ativo do fork `croquete1/ApolloOffice` (issues desativadas na última observação). Regras de desenvolvimento e restrições permanentes: [AGENTS.md](../AGENTS.md). Histórico integral de entregas, findings, SHA, resultados de CI e decisões: [arquivo de 25/11/2026](archive/operational-roadmap-2026-11-25.md). O arquivo preserva o conteúdo original, incluindo observações que foram posteriormente superadas.

## Situação e prioridades

- 1/44 critérios comprovados (F01, logs); nenhuma fase F01–F17 declarada concluída; implementação integrada não equivale a aceitação completa.
- Main observada `e2e14040203929d029d2211a97264465b201ee35` após #141/#142; CI pós-merge #142 em execução. Baseline anterior `41f90b70baafca03bfb1239e2e218f628f25bece`, [CI pós-merge #137](https://github.com/croquete1/ApolloOffice/actions/runs/45836328664): aprovado em Ubuntu e Windows. [CI pós-merge #139](https://github.com/croquete1/ApolloOffice/actions/runs/45830715082) aprovado em Ubuntu e Windows. Lote anterior #130 → #131 → #132 → #128 → #129 e novo lote #133 → #134 → #135 → #136 → #138 → #139 → #137 integrados sequencialmente. A repetição de CIs verdes não demonstra estabilidade continuada.
- Prioridade P0: reconciliação durável, identidade imutável, SQLite e Steward. Auditoria de `src/drop_recovery.rs` na baseline `41f90b70baafca03bfb1239e2e218f628f25bece`: `retry_manual_with` chama `cache_default_head` antes de `retry_snapshot`; `gh::cs` substitui incondicionalmente o valor e a hora de cache, mesmo quando `commit_retry` não aceita o snapshot. `web.rs` devolve 409 quando a recuperação retorna false, mas a alteração da cache já ocorreu. A regressão existente `remote_entry_points_preserve_the_snapshot_during_io` verifica estado, contador, cancelamento e histórico, não a cache. Ensaio local da instrução SQL de `commit_retry` transcrita, em SQLite com esquema mínimo: mutações de número, t1 e metadados rejeitadas (0 linhas; drop preservado); controlo sem mutação aceite (1 linha; ready). Não é execução Rust nem reprodução integral da corrida. Próxima correção: separar validação de SHA de publicação da cache; publicar apenas após commit aceite e antes de notificar workers, sem substituir observação concorrente mais recente. Investigar todos os escritores/leitores, ordem de locks e geração/frescura; não manter mutex durante await. Estender teste remoto existente para cache anterior, escrita concorrente durante I/O, rejeição da transação e controlo positivo; preservar marcadores CI e cancelamento. Toolchain Rust e DNS GitHub indisponíveis na verificação local; não publicar alteração Rust até validação oficial.
- P0 — consumidor confirmado em `src/pipe_impl.rs::red`: a cache `head:{repo}` é aceite por 600 s e a decisão deixa de bloquear CI quando `h["s"]` e `cr["sha"]` são strings diferentes, sem validar SHA; a própria consulta de `red` também escreve na cache após await. Ensaio local Python 3.13.5/SQLite, SQL de `commit_retry` transcrito e predicado final de `red` traduzido (não execução Rust): cinco cenários passaram as asserções diagnósticas. (1) timestamp alterado rejeita transação e deixa job drop, mas escrita anterior de HEAD B liberta bloqueio do SHA A; (2) escrita apenas após commit preserva cache A quando rejeitado; (3) controlo positivo aceita recuperação e escreve B; (4) publicação pós-commit sem controlo de frescura ainda pode substituir observação concorrente C por B; (5) string `invalid` liberta bloqueio, null não. São reproduções do modelo e do SQL, não prova de uma corrida real nem correção funcional. A implementação deve coordenar ambos os escritores, validar identidade SHA antes de usar/publicar, associar geração por chave a pedidos em voo e impedir substituição por resultado obsoleto; publicar apenas após transação aceite e antes de notificar workers. Não segurar mutex durante await nem introduzir ordem de locks incompatível; preservar o comportamento conservador quando a evidência é inválida ou ausente. Testar ambos os caminhos remotos, duas recuperações concorrentes, marca CI válida e falha de transação, incluindo observação nova durante I/O e controlo positivo. Rust/Cargo/Rustfmt ausentes e DNS de GitHub/static.rust-lang.org indisponível na verificação desta sessão; não publicar Rust sem gates oficiais.
- Release anteriormente publicada: fork-v0.10.15; nenhuma nova release. Interface dos agentes e modelos inalterados. CodeRabbit apenas com allowance incluída confirmada; não solicitar consumo adicional.

## Lote anterior integrado

| PR | Entrega | CI do HEAD integrado | CI pós-merge |
|---|---|---|---|
| [#130](https://github.com/croquete1/ApolloOffice/pull/130) | Regras e histórico do roadmap | [45710148313](https://github.com/croquete1/ApolloOffice/actions/runs/45710148313) verde | [45719324004](https://github.com/croquete1/ApolloOffice/actions/runs/45719324004) verde |
| [#131](https://github.com/croquete1/ApolloOffice/pull/131) | Readiness, encerramento, login e orçamento E2E | [45719523882](https://github.com/croquete1/ApolloOffice/actions/runs/45719523882) verde | [45721623521](https://github.com/croquete1/ApolloOffice/actions/runs/45721623521) verde |
| [#132](https://github.com/croquete1/ApolloOffice/pull/132) | Sondagens independentes, frescura, leases e recuo limitado | [45721683836](https://github.com/croquete1/ApolloOffice/actions/runs/45721683836) verde | [45784491538](https://github.com/croquete1/ApolloOffice/actions/runs/45784491538) verde |
| [#128](https://github.com/croquete1/ApolloOffice/pull/128) | Steward: identidade, cronologia e permissões | [45784566339](https://github.com/croquete1/ApolloOffice/actions/runs/45784566339) verde | [45786620486](https://github.com/croquete1/ApolloOffice/actions/runs/45786620486) verde |
| [#129](https://github.com/croquete1/ApolloOffice/pull/129) | Quota: observações distintas e regressão de consumo | [45786682252](https://github.com/croquete1/ApolloOffice/actions/runs/45786682252) verde | [45788900130](https://github.com/croquete1/ApolloOffice/actions/runs/45788900130): aprovado em Ubuntu e Windows |

Revisões CodeRabbit concluídas sobre as alterações funcionais; findings corrigidos e threads resolvidas. Após sincronização das bases, #131/#132/#128/#129 conservaram os blobs funcionais já revistos; o CodeRabbit indicou ausência de novos ficheiros a rever. Isso não constitui aprovação formal de um novo SHA. A validação integral do CI foi repetida para cada HEAD sincronizado.

## Último lote integrado (26/11/2026)

| PR | Entrega | CI do HEAD integrado | CI pós-merge |
|---|---|---|---|
| [#133](https://github.com/croquete1/ApolloOffice/pull/133) | Balanço do roadmap e prioridade P0 | [45790119347](https://github.com/croquete1/ApolloOffice/actions/runs/45790119347) verde | [45793619118](https://github.com/croquete1/ApolloOffice/actions/runs/45793619118) verde |
| [#134](https://github.com/croquete1/ApolloOffice/pull/134) | Caminhos Git literais, OIDs e testes de CRLF no blob | [45793693721](https://github.com/croquete1/ApolloOffice/actions/runs/45793693721) verde | [45795978968](https://github.com/croquete1/ApolloOffice/actions/runs/45795978968) verde |
| [#135](https://github.com/croquete1/ApolloOffice/pull/135) | Integridade do estado final do snapshot, diretórios e nós especiais; isolamento das fixtures Git e erro real de enumeração | [45804303360](https://github.com/croquete1/ApolloOffice/actions/runs/45804303360) verde | [45807192773](https://github.com/croquete1/ApolloOffice/actions/runs/45807192773) verde |
| [#136](https://github.com/croquete1/ApolloOffice/pull/136) | Sinalização de inventário incompleto, limites UTF-8 e bytes inválidos | [45807268823](https://github.com/croquete1/ApolloOffice/actions/runs/45807268823) verde | [45811820653](https://github.com/croquete1/ApolloOffice/actions/runs/45811820653) verde |
| [#138](https://github.com/croquete1/ApolloOffice/pull/138) | Identidade do commit verificada antes da leitura de árvore/blobs; regressões SHA-1/SHA-256 | [45817343203](https://github.com/croquete1/ApolloOffice/actions/runs/45817343203) verde | [45821577773](https://github.com/croquete1/ApolloOffice/actions/runs/45821577773) verde |
| [#139](https://github.com/croquete1/ApolloOffice/pull/139) | Catálogo free-only: rejeição de identificadores duplicados e preço ambíguo | [45821937857](https://github.com/croquete1/ApolloOffice/actions/runs/45821937857) verde | [45830715082](https://github.com/croquete1/ApolloOffice/actions/runs/45830715082) verde |
| [#137](https://github.com/croquete1/ApolloOffice/pull/137) | Identidade efetiva do checkout, rejeição pré-execução e seleção de commit histórico | [45831056537](https://github.com/croquete1/ApolloOffice/actions/runs/45831056537) verde | [45836328664](https://github.com/croquete1/ApolloOffice/actions/runs/45836328664) verde |

Revisão externa dos HEADs integrados sem findings acionáveis: [#133](https://github.com/croquete1/ApolloOffice/pull/133#issuecomment-7187452110), [#134](https://github.com/croquete1/ApolloOffice/pull/134#issuecomment-7187635146), [#135](https://github.com/croquete1/ApolloOffice/pull/135#issuecomment-7188095174), [#136](https://github.com/croquete1/ApolloOffice/pull/136#issuecomment-7188839647), [#138](https://github.com/croquete1/ApolloOffice/pull/138#issuecomment-7190953277), [#139](https://github.com/croquete1/ApolloOffice/pull/139#issuecomment-7191616399), [#137](https://github.com/croquete1/ApolloOffice/pull/137#issuecomment-7190591504). Findings anteriores corrigidos e threads resolvidas. CIs completos da matriz Ubuntu/Windows; as suites Python de qualidade e adaptadores executam em Ubuntu, não em Windows. Integridade do estado final não equivale a sandbox do sistema operativo.

## Lote integrado #141 e #142 (26/11/2026)

| PR | HEAD integrado | CI do HEAD | Merge | CI pós-merge |
|---|---|---|---|---|
| [#141](https://github.com/croquete1/ApolloOffice/pull/141) | `65cc2de8a1e048de5a5a2455136b844e86fb97db` | [45880853476](https://github.com/croquete1/ApolloOffice/actions/runs/45880853476) verde | `2c956830c141fcf035b31f20abad75e80e6ed4a5` | [45889514999](https://github.com/croquete1/ApolloOffice/actions/runs/45889514999) verde |
| [#142](https://github.com/croquete1/ApolloOffice/pull/142) | `3b31166c09702d9df7e5088c6031bd23bf47786b` | [45889593825](https://github.com/croquete1/ApolloOffice/actions/runs/45889593825) verde | `e2e14040203929d029d2211a97264465b201ee35` | [45890421250](https://github.com/croquete1/ApolloOffice/actions/runs/45890421250) em execução na observação |

- #141 (F02/F09/F16): limites de I/O e inventário, proveniência dos objetos Git e configuração nativa, identidade e redirecionamentos HTTP recusados. [Finding CodeRabbit resolvido](https://github.com/croquete1/ApolloOffice/pull/141#discussion_r4709824248); [evidência dos testes e commit atómico](https://github.com/croquete1/ApolloOffice/pull/141#issuecomment-7201168955).
- #142 (F06/F07): validação fail-closed de catálogo free-only, timestamps, preços não nulos arredondados para zero, chaves duplicadas, identidade e framing CLI/JSON; indisponibilidade HTTP explícita e compatibilidade com referências de chamadas por ID. [CI com regressão histórica](https://github.com/croquete1/ApolloOffice/actions/runs/45871991307) e [correção](https://github.com/croquete1/ApolloOffice/pull/142#issuecomment-7200026149). Nenhum fornecedor real ou faturação testados.
- CI integrado #142 [job Ubuntu](https://github.com/croquete1/ApolloOffice/actions/runs/45889593825/job/140148670365): 77/77 quality, 65/65 OpenCode, Rust 435 aprovados/5 ignorados e 2 testes de integração; Docker e SSH reais 1/1 cada em steps separados. Windows verde nos gates aplicáveis. Os três restantes testes ignorados requerem fornecedor OpenCode real autorizado: `opencode_author_repairs_repo_via_apollo_host_tools`, `opencode_remote_worker_resolves_home_without_faking_verification`, `opencode_repro_and_fixer_use_production_protocol`. Não equivalem a aceitação real.
- Integrações autorizadas expressamente pelo proprietário com CI integral verde, base atual, sem conflitos nem findings conhecidos por resolver, sem aguardar revisão independente opcional do HEAD. Não houve aprovação independente dos HEADs finais; não apresentar como tal. Não alterar os gates internos de Guard/Reviewer/Steward, proteções GitHub ou política de custos.

## Frente aberta

- #140: consolidação documental e reconhecimento de um critério F01 comprovado pela main. Integrar somente após CI pós-merge #142 e CI do HEAD sincronizado verdes. Priorizar critérios reais de aceitação F01–F04; não ampliar indefinidamente alterações já integradas. 1/44 critérios comprovados neste documento, 0/17 fases completas.

## Balanço por fase

Evidência integrada consolidada a partir do arquivo e dos lotes até #139, incluindo #137 integrada. Não atribuir a main alterações apenas propostas.

| Fase | Implementação parcial integrada (referências principais) | Trabalho por concluir |
|---|---|---|
| F01 / P0 | #49, #67, #78: Docker, fnm, logs e CI isolado; 1/4 critérios comprovados | Workers reais, seleção Node, retoma e caches |
| F02 / P0 | #48/#61/#78/#92–94/#101–103/#115/#120/#125/#134–138: admissão, outbox, recibos, recuperação conservadora e evidência de inspeção | Inbox integral, identidade remota imutável, reconciliação autorizada e restart real |
| F03 / P0 | #52/#63/#78/#90–91/#95/#127–128: Steward, inventários, identidades e autorizações | Centralização integral e testes remotos de concorrência e revogação |
| F04 / P0 | #51/#57/#62/#79–97/#101/#106/#118/#120/#125: transações, rollback, barreiras e holds | Escritores restantes, disco cheio, falha simultânea BD/disco, diagnóstico e recuperação real |
| F05 / P1 | #54/#62/#78/#88–89/#103–104/#116/#118/#131–132: checkpoints, retoma, cancelamento e validade de rede | Chamadas de fornecedores, recuperação de processo, efeitos incertos e estabilidade recorrente |
| F06 / P1 | #53/#58/#78/#139/#142: cooldowns, routing autorizado e rejeição de metadados duplicados free-only | Circuit breakers restantes e fornecedores reais |
| F07 / P1 | #53/#56/#78/#121/#129: reserva, histórico transacional e observações distintas da quota | Identidade da conta, calibração e limites concorrentes reais |
| F08 / P1 | #58: routing adaptativo opt-in do Fixer | Restantes papéis, calibração e eficácia medida |
| F09 / P1 | #51/#52/#63/#78/#91–94/#97/#120/#125/#127/#134/#136/#138/#141: paginação, snapshots, inventários conservadores e identidade de commit | Sync incremental, mutações remotas e identidade após recriação |
| F10 / P1 | #54/#56/#62/#78/#79/#82/#89–90/#97/#103: prioridades e dependências | Causa comum, agrupamento e cobertura integral de starvation |
| F11 / P1 | #56/#78/#108/#122/#124: perfis, CI, telemetria e limites disco | Pressão em hardware real e prioridade de processos |
| F12 / P1 | #61/#62/#78/#135/#137: mitigação de credenciais, Docker/SSH isolados e integridade final do snapshot | Isolamento por confiança/repo, caches executáveis e workers reais |
| F13 / P2 | #59: aprendizagem supervisionada | Eficácia, expiração e interação após reinício |
| F14 / P2 | #56/#122/#124: perfis e limites adaptativos | Transições, persistência e pressão em máquinas reais |
| F15 / P2 | #56/#58/#59/#78: diagnóstico e logs | Próxima tarefa elegível, histórico integral e regressão visual |
| F16 / P2 | #58/#59/#96/#98/#105/#110–111/#117/#121/#134–138: métricas, assinatura, staging e evidência de qualidade | Anti-rollback persistente, chave de produção, instalação/recuperação e métricas comparáveis |
| F17 / P1 | #78 (inclui #76/#77): política, evidência consultiva e persistência | Confirmar OSS após trial, integração integral de findings e recuperação end-to-end |

## Fases e critérios de aceitação
### F01 / P0 — Integração seletiva upstream
- [ ] Validar #49: init, limites CPU daemon, stderr e contentores pausados; testes de retoma, volumes e cache npm. Sem recriar contentores com jobs ativos.
- [ ] Portar fnm (upstream c464574) mantendo Node 24 do sistema por omissão; versão por repo sem alterar default partilhado. Testar repo Node 20 seguido de repo sem versão, versão inválida e configuração de setup.
- [x] Adaptar filtros/pesquisa e wrap dos logs do upstream à interface operacional. Integrado na [#67](https://github.com/croquete1/ApolloOffice/pull/67) (merge `1feb1d510e4cd42f1e60542586da4052223dc746`); validado na main `41f90b70baafca03bfb1239e2e218f628f25bece` pelo [CI pós-merge 45836328664](https://github.com/croquete1/ApolloOffice/actions/runs/45836328664), cinco testes de `quality/job-log-filters.test.mjs` aprovados: pesquisa combinada, contagem/escape/wrap, refresh com preservação de filtros e falha de rede, deduplicação de refresh e sessão expirada. Não equivale a concluir a regressão visual integral F15.
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
- Toolchain Rust local indisponível na sessão de 26/11: cargo, rustc e rustfmt não encontrados; resolução DNS de static.rust-lang.org falhou. Não publicar novas alterações Rust até dispor de validação oficial; trabalho Python/documental independente pode avançar.
- Quota e identidade Codex: consulta atual não fornece identificador de conta; não misturar históricos por inferência nem ler credenciais.
- CodeRabbit: confirmar elegibilidade gratuita após trial indicado para 08/12/2026, faturação OFF e instalação limitada a repositórios públicos autorizados. Última configuração registada: auto/incremental OFF, drafts/status ON, Assertive, pt-PT; revalidar antes de pedidos. Consultar evidências e critérios complementares F17 no arquivo (secção Registo complementar da PR #77).
- Produção: testes reais de workers, hardware, fornecedores, restart, instalação/rollback e chave de confiança ainda necessários; ensaios isolados de CI não são validação de produção.

## Atualização do documento

Manter apenas o estado atual, frentes abertas, critérios de aceitação, bloqueios e referências a evidências. Arquivar histórico cronológico sem apagar resultados negativos. Não marcar critérios como concluídos sem prova; consultar AGENTS.md para o processo e atualizar esse ficheiro quando surgir uma regra permanente reutilizável.
