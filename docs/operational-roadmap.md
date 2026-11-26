# Apollo Office — Roadmap operacional

Atualizado em 26/11/2026. Este é o backlog ativo do fork `croquete1/ApolloOffice` (issues desativadas na última observação). Regras de desenvolvimento e restrições permanentes: [AGENTS.md](../AGENTS.md). Histórico integral de entregas, findings, SHA, resultados de CI e decisões: [arquivo de 25/11/2026](archive/operational-roadmap-2026-11-25.md). O arquivo preserva o conteúdo original, incluindo observações que foram posteriormente superadas.

## Situação e prioridades

- 6/44 critérios comprovados no âmbito de aceitação (1/17 fases): F01 4/4; F02 2/3. F02 continua aberta: separar explicitamente a tentativa em análise da espera de retry na persistência. Validação de fornecedores reais e produção continua em F06/F12/F16.
- Main observada `96df2f7e9e8f0664def9763a371ae3c4cdbb655f` após #146–149; CI pós-merge #149 [45906415696](https://github.com/croquete1/ApolloOffice/actions/runs/45906415696) em execução na observação. CI pós-merge #142 aprovado. Baseline anterior `41f90b70baafca03bfb1239e2e218f628f25bece`, [CI pós-merge #137](https://github.com/croquete1/ApolloOffice/actions/runs/45836328664): aprovado em Ubuntu e Windows. [CI pós-merge #139](https://github.com/croquete1/ApolloOffice/actions/runs/45830715082) aprovado em Ubuntu e Windows. Lote anterior #130 → #131 → #132 → #128 → #129 e novo lote #133 → #134 → #135 → #136 → #138 → #139 → #137 integrados sequencialmente. A repetição de CIs verdes não demonstra estabilidade continuada.
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
| [#142](https://github.com/croquete1/ApolloOffice/pull/142) | `3b31166c09702d9df7e5088c6031bd23bf47786b` | [45889593825](https://github.com/croquete1/ApolloOffice/actions/runs/45889593825) verde | `e2e14040203929d029d2211a97264465b201ee35` | [45890421250](https://github.com/croquete1/ApolloOffice/actions/runs/45890421250) verde |

- #141 (F02/F09/F16): limites de I/O e inventário, proveniência dos objetos Git e configuração nativa, identidade e redirecionamentos HTTP recusados. [Finding CodeRabbit resolvido](https://github.com/croquete1/ApolloOffice/pull/141#discussion_r4709824248); [evidência dos testes e commit atómico](https://github.com/croquete1/ApolloOffice/pull/141#issuecomment-7201168955).
- #142 (F06/F07): validação fail-closed de catálogo free-only, timestamps, preços não nulos arredondados para zero, chaves duplicadas, identidade e framing CLI/JSON; indisponibilidade HTTP explícita e compatibilidade com referências de chamadas por ID. [CI com regressão histórica](https://github.com/croquete1/ApolloOffice/actions/runs/45871991307) e [correção](https://github.com/croquete1/ApolloOffice/pull/142#issuecomment-7200026149). Nenhum fornecedor real ou faturação testados.
- CI integrado #142 [job Ubuntu](https://github.com/croquete1/ApolloOffice/actions/runs/45889593825/job/140148670365): 77/77 quality, 65/65 OpenCode, Rust 435 aprovados/5 ignorados e 2 testes de integração; Docker e SSH reais 1/1 cada em steps separados. Windows verde nos gates aplicáveis. Os três restantes testes ignorados requerem fornecedor OpenCode real autorizado: `opencode_author_repairs_repo_via_apollo_host_tools`, `opencode_remote_worker_resolves_home_without_faking_verification`, `opencode_repro_and_fixer_use_production_protocol`. Não equivalem a aceitação real.
- Integrações autorizadas expressamente pelo proprietário com CI integral verde, base atual, sem conflitos nem findings conhecidos por resolver, sem aguardar revisão independente opcional do HEAD. Não houve aprovação independente dos HEADs finais; não apresentar como tal. Não alterar os gates internos de Guard/Reviewer/Steward, proteções GitHub ou política de custos.

## Lote F01 integrado

| PR | HEAD integrado | CI do HEAD | Merge | CI pós-merge |
|---|---|---|---|---|
| [#140](https://github.com/croquete1/ApolloOffice/pull/140) | `1e444737904d861823fd42e3526493e426554bdc` | [45890544053](https://github.com/croquete1/ApolloOffice/actions/runs/45890544053) verde | `e4c5d2d5de9a8e5147425850553c125f5a75da2e` | [45891506206](https://github.com/croquete1/ApolloOffice/actions/runs/45891506206) verde |
| [#143](https://github.com/croquete1/ApolloOffice/pull/143) | `537ed8882dcb35d4bb469d6b807585ef98e6f31c` | [45892313287](https://github.com/croquete1/ApolloOffice/actions/runs/45892313287) verde | `b167b653cdc57acd8991b9459ccffbfe773945b0` | [45892950005](https://github.com/croquete1/ApolloOffice/actions/runs/45892950005) verde |
| [#144](https://github.com/croquete1/ApolloOffice/pull/144) | `7678a87b6483defa594e089d1110c36f2c814edd` | [45893200098](https://github.com/croquete1/ApolloOffice/actions/runs/45893200098) verde | `5058224bebce12bc472c780be0328380f9e417a3` | [45900738459](https://github.com/croquete1/ApolloOffice/actions/runs/45900738459) verde |

- #143: confiança Ed25519 independente antes de Git/build/publicação; validação da assinatura real contra chave de confiança e restrição dos três jobs do workflow upstream ao repositório upstream. Testes usam chaves OpenSSL temporárias; GitHub não é chamado pelo publicador nas regressões. Sem criação de tag/release.
- #144: execução em Docker real do código shell de seleção existente, Node 20 -> Node 24, versão inválida e setup isolado. [Log Ubuntu](https://github.com/croquete1/ApolloOffice/actions/runs/45893200098/job/140160646785): 435 Rust aprovados, 5 ignorados na suite principal, quality 83/83, OpenCode 65/65; Docker/SSH reais em passos separados verdes. Não é execução completa de um job remoto com fornecedor real.
- F01: concluir os quatro critérios no âmbito dos ensaios de integração seletiva; manter produção, hardware, instalação/rollback e fornecedores reais em F12/F16. Prioridade seguinte F02: inbox persistente, recuperação/reconciliação e testes end-to-end; não reabrir correções F01 sem falha material.

## Lote F02 integrado

| PR | Entrega | CI do HEAD integrado | Merge | CI pós-merge |
|---|---|---|---|---|
| [#146](https://github.com/croquete1/ApolloOffice/pull/146) | SQLite WAL, interrupções e reabertura | [45901982429](https://github.com/croquete1/ApolloOffice/actions/runs/45901982429) verde após rerun Windows | `b21970985d92e80525f63f4ce5321bbe94f54c08` | [45902886457](https://github.com/croquete1/ApolloOffice/actions/runs/45902886457) verde |
| [#147](https://github.com/croquete1/ApolloOffice/pull/147) | Reinício real do Watcher/outbox | [45903813480](https://github.com/croquete1/ApolloOffice/actions/runs/45903813480) verde | `7097610c449435eaf2c9e046a68c45fb5f2245d8` | [45904501387](https://github.com/croquete1/ApolloOffice/actions/runs/45904501387) verde |
| [#148](https://github.com/croquete1/ApolloOffice/pull/148) | Reinício real do Watcher/inbox | [45904802597](https://github.com/croquete1/ApolloOffice/actions/runs/45904802597) verde | `8e593e47da5ac2ff28608ccec98316d4cea66cb3` | [45905748997](https://github.com/croquete1/ApolloOffice/actions/runs/45905748997) verde |
| [#149](https://github.com/croquete1/ApolloOffice/pull/149) | Resultado Claude exige conclusão e exit válidos | [45905951655](https://github.com/croquete1/ApolloOffice/actions/runs/45905951655) verde, Rustfmt corrigido | `96df2f7e9e8f0664def9763a371ae3c4cdbb655f` | [45906415696](https://github.com/croquete1/ApolloOffice/actions/runs/45906415696) em execução na observação |

- #146: nove checkpoints na camada SQL (não execução Rust); timeout intermitente dos dois E2E Windows no primeiro run, rerun aprovado sem alterar timeout. Causa-raiz não demonstrada. #147–149: binário Linux real, GitHub e fornecedor simulados em rede isolada, sem credenciais nem chamadas reais. #149: 436 testes Rust aprovados/5 ignorados na suite principal; 2 E2E, 86 testes Python quality, 65 OpenCode; Docker/SSH reais, Rustfmt e Clippy aprovados. Nove checkpoints da inbox e dois da outbox aprovados. Revisão externa não obtida neste lote; merge segundo AGENTS §5, sem pedido de review com allowance incerta.
- F02: a reserva `watcher_retry` é persistida antes de chamar Guard/Watcher e mantém o prazo após crash. `watcher_inbox` distingue `received`, `done`, `superseded`, mas não distingue durablemente tentativa em análise de espera de retry. Não marcar o primeiro critério como concluído sem resolver esta lacuna. Próxima implementação: transição de tentativa explícita, identificação da tentativa e recuperação segura de leases após crash, sem modificar os gates de admissão nem antecipar o backoff; verificar migração legada, concorrência e todos os caminhos de retorno/erro. Os restantes dois critérios têm evidência integrada. Não repetir ensaios já aprovados sem regressão material.

## Balanço por fase

Evidência integrada consolidada a partir do arquivo e dos lotes até #139, incluindo #137 integrada. Não atribuir a main alterações apenas propostas.

| Fase | Implementação parcial integrada (referências principais) | Trabalho por concluir |
|---|---|---|
| F01 / P0 | #49, #67, #78, #143, #144: 4/4 critérios comprovados no âmbito F01 | Encerrada no âmbito F01; aceitação em produção permanece em F12/F16 |
| F02 / P0 | #48/#61/#78/#92–94/#101–103/#115/#120/#125/#134–138/#146–149: 2/3 critérios comprovados, recuperação real do processo com serviços simulados | Estado persistente em análise vs retry, transições e recuperação de tentativa |
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
- [x] Validar #49: init, limites CPU daemon, stderr e contentores pausados; testes de retoma, volumes e cache npm. Sem recriar contentores com jobs ativos. [#49 integrada](https://github.com/croquete1/ApolloOffice/pull/49); [CI #144, Ubuntu](https://github.com/croquete1/ApolloOffice/actions/runs/45893200098/job/140160646785): `docker_cpu_cap_respects_daemon_ceiling`, `worker_image_build_must_succeed_before_replacing_container`, `failed_image_build_does_not_remove_existing_container`, `failed_docker_limit_update_is_not_reported_as_success`, `utf8_stderr`, `cache_cleanup_only_targets_npm_downloads_after_limit`, `npm_cache_cleanup_only_reports_success_after_bounded_npx_removal`, `orphaned_npm_execution_caches_are_reclaimed_only_when_idle` aprovados. `real_docker_worker_lifecycle` executado separadamente e aprovado: pausa, retoma, volume de trabalho e cache npm. `idle_cache_cleanup_blocks_admission_and_releases_on_drop` e `cache_maintenance_serializes_aliases_of_the_same_docker_target` aprovados; `ensure` usa a mesma reserva antes de operações Docker. A proteção de jobs ativos foi validada pela reserva/admissão unitária e inspeção do caminho de chamada, não por corrida de recriação num worker de produção; escopo limitado a jobs geridos e aliases conhecidos. F12 permanece aberta.
- [x] Portar fnm (upstream c464574) mantendo Node 24 do sistema por omissão; versão por repo sem alterar default partilhado. Testar repo Node 20 seguido de repo sem versão, versão inválida e configuração de setup. [#144 integrada](https://github.com/croquete1/ApolloOffice/pull/144), [CI 45893200098, Ubuntu](https://github.com/croquete1/ApolloOffice/actions/runs/45893200098/job/140160646785): `F01: Node 20, Node 24, invalid version and isolated setup checks passed`. Docker/fnm reais, constantes shell de produção, override explícito e shell seguinte Node 24; não é ensaio integral do pipeline remoto.
- [x] Adaptar filtros/pesquisa e wrap dos logs do upstream à interface operacional. Integrado na [#67](https://github.com/croquete1/ApolloOffice/pull/67) (merge `1feb1d510e4cd42f1e60542586da4052223dc746`); validado na main `41f90b70baafca03bfb1239e2e218f628f25bece` pelo [CI pós-merge 45836328664](https://github.com/croquete1/ApolloOffice/actions/runs/45836328664), cinco testes de `quality/job-log-filters.test.mjs` aprovados: pesquisa combinada, contagem/escape/wrap, refresh com preservação de filtros e falha de rede, deduplicação de refresh e sessão expirada. Não equivale a concluir a regressão visual integral F15.
- [x] Preservar canal Ed25519 exclusivo e não importar tags/releases upstream. [#143 integrada](https://github.com/croquete1/ApolloOffice/pull/143), [CI pós-merge 45892950005](https://github.com/croquete1/ApolloOffice/actions/runs/45892950005) verde: proteção de confiança independente e restrição do workflow upstream. Inventário observado em 26/11: 9 releases publicadas, todas `fork-v*`, cada uma com binário Linux x64, `latest.json` e `latest.json.sig`. As 24 tags legadas `v*` já presentes permanecem intocadas; nenhuma importação de tags ou releases durante este lote. Não verificadas assinaturas de releases históricas contra a chave de produção; instalação, rollback e chave real continuam em F16.

### F02 / P0 — Pedidos de revisão duráveis
- [ ] Validar #48; introduzir inbox persistente com recebido/em análise/concluído/retry e next_attempt_at. #48 integrada; recibos por repo/PR/HEAD/ID e reserva de retry já implementados. `retry_window_is_persistent_and_snapshot_scoped` valida janela e reset por snapshot. Falta distinguir explicitamente em análise vs retry; manter critério aberto.
- [x] Não perder pedidos quando um job está ativo ou um fornecedor falha. Deduplicar por repo/PR/HEAD/review-id, conservar evidência e recuperar após restart. Não confirmar uma ação que falhou. [CI #149](https://github.com/croquete1/ApolloOffice/actions/runs/45905951655): provider-failure, failed-terminal, failed-exit, duplicate-terminal, active-job, enqueue-failure, recovery, replay aprovados com binário real e CLI simulados. Recibo/HEAD/ID/retry preservados; nenhuma confirmação ou job indevido; recuperação cria um único job com source_head/source_comments. A chave do recibo inclui HEAD; o ledger de acknowledgements por repo/ID evita repetir comentários consumidos. Corpos são relidos da fonte, não persistidos nem tratados como autorização. Sem teste de disponibilidade de fornecedor real.
- [x] Teste end-to-end de falha após leitura antes de Guard, após análise, após enqueue e durante confirmação; não publicar resposta duplicada. [#146](https://github.com/croquete1/ApolloOffice/pull/146): checkpoints SQL com saída abrupta. [#147](https://github.com/croquete1/ApolloOffice/pull/147) e [#148/#149, CI final](https://github.com/croquete1/ApolloOffice/actions/runs/45905951655): SIGKILL durante Guard antes do veredicto, falha de fornecedor, falha de ack via trigger SQLite após inserção de job, reinício e replay. Outbox: POST simulado com resposta perdida, efeito presente -> sent, efeito ausente -> uncertain; exatamente uma tentativa por cenário. Usa binário real, base temporária e rede isolada; avança prazos na fixture parada, não mede o backoff em tempo real. Não equivale a validação em produção.

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
