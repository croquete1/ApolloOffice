# ApolloOffice — Roadmap operacional (24/11/2026)

## Requisitos do proprietário
- Preservar integralmente a interface atual dos agentes, incluindo canvas pixelizado, personagens, disposição, cores, animações e interações existentes, até autorização expressa para a alterar. Não substituir por grelha, novo cenário ou outra apresentação. Melhorar apenas os restantes módulos operacionais sem interferir com essa interface.
- Qwen não está em utilização. Não selecionar, iniciar, instalar ou exigir Qwen/Ollama no plano de trabalho. Manter compatibilidade opcional existente; apenas usar modelos e fornecedores explicitamente configurados pelo proprietário.
- Implementar todas as melhorias da auditoria incrementalmente, preservando autorização, revisão independente, provas RED/GREEN, CI, atualização assinada e configurações.
- As issues estão desativadas no fork (GitHub HTTP 410); este ficheiro constitui o backlog versionado. Não declarar uma fase concluída sem evidência.

## Estado em 24/11/2026
- Release publicada: fork-v0.10.15; nenhuma nova release ou atualização automática foi publicada.
- PR #47 — encerrada sem merge a pedido do proprietário; interface dos agentes preservada.
- PRs #48, #49, #51, #52, #53 e #54 integradas conjuntamente através da PR #55. Merge da main: d453bf72ba9dcf14b5ebc032165680c53f4f1c00. Árvore de código validada: 754be0a217c5139b24faeb19e1ed134ff69ca69c.
- #48: confirmação transacional de comentários e encaminhamento de novos merges para o Steward.
- #49: melhorias Docker, fnm e seleção de Node por repositório.
- #51: paginação Scout e snapshot SQLite transacional.
- #52: inventário paginado do Steward e paginação de threads de revisão.
- #53: isolamento do limite Claude em instalações mistas e reserva configurável de quota Codex.
- #54: cooldown persistente de falhas do host, prioridades manuais, dependências acíclicas, equilíbrio entre projetos e diagnóstico API. Encerrada como redundante após confirmar ancestralidade na main; o GitHub não permitiu alterar a base para main porque não existiam novos commits.
- QA Linux da árvore integrada: cargo test --locked (225 unitários + 2 integração aprovados, 3 ignorados); node --test (116 aprovados); Python quality/adapters (5 + 29 aprovados); rustfmt, Clippy -D warnings e git diff --check aprovados. Total: 377 testes aprovados.
- Ainda não há evidência de execução CI no GitHub. Validação real Docker, Windows, Chromium e fornecedores reais pendentes. Nenhuma fase completa do roadmap é declarada concluída só por estes resultados.
- Interface dos agentes inalterada; não foi ativado Qwen.

## Segunda entrega — 24/11/2026
- [x] PR #56 — perfis Manual/Económico/Equilibrado/Desempenho/Noturno, redução de novas admissões por CPU/memória/temperatura e telemetria indisponível, reserva efetiva de lane para fixes, controlos de prioridade e dependências no drawer, configuração da reserva Codex. Merge dec21d3e6c465b157f85d79a4945372fa91c4a3f. QA: 379 testes aprovados, fmt/Clippy, Chromium desktop e mobile.
- [x] PR #57 — migrações SQLite transacionais versionadas, idempotência, rollback, preservação de permissões, leituras estritas de fila e prova de merge, pausa por falha de ledger global. Merge c08d2fcdc76f02885f8253335ed3873dc4470e8e. QA: 382 testes aprovados, fmt/Clippy.
- [x] PR #58 — routing adaptativo opt-in apenas do Fixer normal, modelos de subscrição explicitamente definidos em Fixer/Big, disponibilidade, distinção do Reviewer, histórico de primeiras tentativas por repo (mínimo 5 amostras por modelo, diferença >=15 pontos percentuais), diagnóstico. Merge 94961fc95e75fed5545386a2fa6ab2f8320e4aef. QA: 384 testes aprovados, fmt/Clippy, Chromium desktop e mobile.
- [x] PR #59 — aprendizagem supervisionada opt-in, proposta/evidência/proveniência, regra escrita e aprovada pelo proprietário, âmbito de repo, validade e revogação; métricas de primeira tentativa por projeto/modelo no painel Custos. Merge 739058ed84e1c21e44bbf5c6d9e59e635ad02e5a. QA: 385 testes aprovados (233 Rust unitários + 2 integração + 116 JavaScript + 34 Python; 3 Rust ignorados), fmt, Clippy -D warnings, diff check e Chromium desktop idle/active e mobile PASS. Árvore testada 5c438d66870b43d17fee60866748a9a0ab028b06 idêntica à integrada.
- Interface dos agentes preservada; alterações UI restritas a definições, detalhes de tarefa, aprendizagem e custos. Não ativado Qwen. Perfis Manual, routing adaptativo OFF e aprendizagem supervisionada OFF mantêm comportamento preexistente até opção do proprietário. Nenhuma release nova.
- Windows, Docker real, fornecedores reais, testes interativos específicos dos novos formulários e impacto medido das lições continuam pendentes. Smoke Chromium existente usa estado sintético, não equivale a validação de produção.

## Terceira entrega — 24/11/2026
- [x] PR #61 — pedidos de seguimento pós-merge com admissão/ack transacionais, replays idempotentes, pendência quando outro follow-up está ativo, cursor não avançado em falha de análise; proteção de manutenção Docker e limpeza de órfãos com reserva idle; impedir reconfiguração de hosts durante trabalho; remover variáveis conhecidas de credenciais de workers locais e desativar forwarding SSH. Merge 251450818141d4f56477e72e4aba75edae9437d5. QA: 387 testes aprovados, fmt/Clippy/diff check. F02/F12 parciais; não constitui fronteira de segurança por repo nem isolamento de credenciais em hosts raw/SSH.
- [x] PR #62 — espera diferida persistente em fronteiras seguras de rede e capacidade CI para candidatos verificados, validação de SHA e nova execução da prova na retoma, reserva de checkout por alias, manutenção de aprovação e gate explícito de proprietário para fixes em modo ask também após recovery; limites por repo fail-closed e bloqueio antes de admissão. CI após rebase no gate mantém espera não diferível para não permitir publicação sem testes. Merge 2ab59a2a082d28ea25da0c0de7428ca793a028f1. QA: 390 testes aprovados, fmt/Clippy/diff check. F04/F05/F10 parciais: esperas durante chamadas de fornecedores, fases sem checkpoint seguro e simulação completa de recuperação ainda pendentes.
- [x] PR #63 — inventários paginados de check-runs, status combinado, revisões/comentários e permissões GraphQL de revisores; falha fechada perante contagem inconsistente, IDs duplicados, páginas incompletas, erro GraphQL ou cursor repetido. Bloqueio de renomeações de ficheiros sensíveis; limite conservador de tamanho de PR preservado. Merge 621d1e8372994745105777a1f8bb45f54c0c3395. QA final: 395 testes aprovados (243 Rust unitários + 2 integração + 116 JavaScript + 34 Python), 3 Rust ignorados, fmt, Clippy -D warnings e diff check aprovados. Árvore testada b17695d9fca2190058ac048a82604fbb92f6b934 idêntica à integrada. F03/F09 parciais; API real com várias páginas e sync incremental ainda por validar/implementar.
- Interface dos agentes e configuração dos modelos inalteradas. Sem ativação Qwen e sem nova release. Browser validado na segunda entrega, não repetido nesta etapa sem alterações UI; Windows, Docker/SSH real e fornecedores reais continuam pendentes.

## Quarta entrega — desenvolvimento pendente de validação (24/11/2026)
Estas PRs estão em draft e **não estão integradas**. A main funcional continua na árvore anteriormente validada; a contagem de 395 testes refere-se à terceira entrega.
- #65 — F06/F07: cooldowns persistentes por fornecedor e reserva Codex para otimizações; três testes Rust criados, não executados. HEAD c645240fcca01e73af63e61dcca12afa919b46b5.
- #66 — F09/F10: inventários completos Hunter, validação de evidência, supressão atómica de duplicados exatos e aging automático até P1; quatro testes Rust criados, não executados. SQLite isolado PASS, incluindo concorrência com oito ligações. HEAD f9fef8c08ff30880d6724c7eb84d8b962b0bfd29.
- #67 — F01/F15: filtros de logs por agente e mensagem, contagem e quebra de linhas no drawer de tarefas; dois testes JS isolados PASS, browser e suite integral pendentes. HEAD f0eee616dc04f75faed4525c6c98f0eeae10538f (ampliada na nona entrega).
- #68 — F11/F14/F15: capacidade de CI adaptada aos perfis e pressão observada; tarefas retomáveis podem adiar CI, mantendo um limite mínimo de uma vaga para concluir trabalho não retomável; GPU/VRAM/temperatura/utilização NVIDIA opcionais, sem drivers novos, sem dependência de Ollama ou Qwen; diagnóstico no drawer Health, dados stale omitidos. Dois testes Rust criados, não executados. Quatro cenários do probe shell com utilitário simulado e dois testes JS isolados do renderer PASS, hashes dos trechos e teste confirmados. HEAD 6777ef95eef51dabcc6e77134b087994188ca41e. Não é validação em GPU real nem suporta ainda telemetria AMD/Intel.
- Bloqueio: limite Hobby Vercel anteriormente atingido; ambiente local sem cargo/rustc e sem resolução DNS GitHub; sem runs CI verificados. Não declarar fase encerrada ou fazer merge funcional até resolver conflitos, formatar e validar a árvore conjunta. Preservar interface dos agentes e configuração dos modelos.

## Quinta entrega — revisão em desenvolvimento (24/11/2026)
- PR #70 (draft) — F02/F03/F04: isolamento de falhas por PR; validação de metadados e timestamps, atualização condicional preservando o cursor e recusando snapshots concorrentes desatualizados; scans com falha não avançam o throttle de sucesso; admissão transacional de revisões inéditas com PR aberta, exclusão perante revisão/rebase/recheck ativo, registo dos comentários de origem. HEAD c3e43db9eea912e7750b6e47758ebdca944f357c.
- QA SQLite isolado com Python: cinco cenários PASS (8 ligações concorrentes, uma inserção; replay integral/parcial; bloqueio por rebase e recuperação; preservação de cursor e rejeição de snapshot stale; PR encerrada). Dois testes Rust novos e extensão de teste existente, não executados. Não constitui compilação, teste integral do Watcher ou validação da API GitHub.
- Inbox/outbox integral, HEAD remoto na admissão, exclusão universal de operações concorrentes e testes Rust permanecem pendentes. As PRs #65–70 funcionais (#65, #66, #67, #68, #70) não foram integradas; #69 é documentação já integrada. Última bateria integral validada: 395 testes. Interface dos agentes e configuração de modelos preservadas.

## Sexta entrega — outbox persistente (24/11/2026)
- PR #70 ampliada (draft, HEAD c7f13b0267d782fa337b87efaf7b00a5e5fcbaa6): migração SQLite v2, respostas do Watcher gravadas na mesma transação que a confirmação dos comentários e, quando aplicável, a tarefa de revisão. Identificador determinístico por lote; claim atómico pending→uncertain antes de POST; reconciliação por autor e corpo exatos, sem reenvio automático perante resultado ambíguo. Identidade GitHub verificada antes do envio; expiração de 24h para respostas ainda não enviadas, cancelamento se PR encerrada, limite anterior de respostas preservado; contadores agregados em ops.watcher_outbox, sem expor conteúdo. Decisão conservadora: uma resposta incerta pode necessitar de intervenção humana, mesmo quando nunca chegou ao GitHub.
- QA desta entrega: seis cenários SQLite executados com Python PASS (rollback, repetição de chave, identidade incorreta, oito claims concorrentes com um vencedor, reabertura de DB e confirmação sem reenvio, cancelamento). Três novos testes Rust escritos, não executados. Não equivale a validação integral da implementação, da migração em Rust ou da API remota.
- Pendentes: interface de resolução manual, inbox persistente completa, vinculação a HEAD remoto, validação real de entrega GitHub e testes de integração das cinco PRs funcionais #65–68 e #70. Nenhum merge funcional novo; última suite integral validada continua 395. Agentes e configuração dos modelos inalterados.

## Sétima entrega — diagnóstico e resolução de entregas (24/11/2026)
- PR #70 ampliada (draft, HEAD ab858f7f7f3c9ea371cb92b9353f4f8597a83908): inventário paginado da outbox com pendentes/incertas primeiro e contadores agregados; painel de proprietário acessível a partir de Health, conteúdo escapado, cancelamento explícito apenas antes do claim e verificação de entrega incerta mediante correspondência exata de autor e corpo nos comentários remotos. API autenticada existente com proteção de origem nos POST. Registos terminais só de leitura. Não permite reenviar automaticamente, cancelar envio já reclamado nem marcar uma entrega como confirmada sem evidência.
- QA isolado desta entrega: 2 testes Node PASS (ações permitidas por estado e escaping; hashes Git do renderer e teste correspondem ao código publicado); 3 verificações SQLite PASS (ordenação/offset, cancelamento após claim impedido, contagem por estado). Um novo teste Rust escrito, não executado. API e interação browser integral ainda não validadas.
- Limites: offset sobre inventário mutável pode mudar entre páginas; botão atualizar permite reconciliar a visualização. Entregas incertas sem correspondência continuam incertas; não há override nem novo envio. Inbox completa, vinculação a HEAD remoto, integração com restantes PRs e QA real continuam pendentes. Cinco PRs funcionais em draft, sem novos merges funcionais; última suite integral validada continua 395. Sem alterações à representação dos agentes nem aos modelos.

## Oitava entrega — vinculação ao HEAD das revisões (24/11/2026)
- PR #70 ampliada (draft, HEAD aa83e047827fe7d8a139459c1af888b0a146dc03): Watcher captura SHA e snapshot da PR, revalida identidade do ramo/repo, timestamp e estado de revisão após análise; recusa inventário de ficheiros incompleto. Admissão e ack exigem snapshot local aberto, timestamp e HEAD exatos na transação. Tarefa guarda SHA/timestamp de origem e comentários. Fixer verifica commit obtido e prova de origem na retoma/publicação; antes de publicar, verifica novamente PR aberta, repo, ramo e SHA remoto, mantendo force-with-lease. Steward associa SHA de origem e exclui novas revisões perante rebase/recheck ativo.
- Decisões: revisões legadas sem SHA mantêm comportamento anterior; trabalho admitido cujo HEAD mudou é interrompido com diagnóstico e pedido original no histórico, exigindo intervenção manual. Não constitui inbox com recuperação automática nem elimina todas as corridas remotas. Inventário de ficheiros incompleto bloqueia, não aumenta limites silenciosamente.
- QA: seis cenários SQLite isolados executados PASS sobre a condição de snapshot (correspondência exata, HEAD alterado, timestamp alterado, âmbito repo/PR, PR encerrada, JSON inválido); quatro testes Rust adicionais escritos, não executados. Vercel novamente devolveu 402, reposição 01/12/2026 00:00 UTC; cargo/rustc indisponíveis localmente. Nenhuma alteração funcional integrada. Cinco drafts continuam pendentes, última suite integral validada 395. Interface dos agentes e modelos preservados.

## Nona entrega — atualização de logs sem perder contexto (24/11/2026)
- PR #67 ampliada (draft, HEAD f0eee616dc04f75faed4525c6c98f0eeae10538f): atualização manual apenas dos logs da tarefa, preservando pesquisa, agente selecionado, quebra de linhas e formulário de aprovação; um pedido em curso por painel; rejeição de respostas de um painel substituído; último registo conservado quando a atualização falha.
- QA: quatro cenários de regressão (dois anteriores e dois novos) PASS sobre o helper de produção obtido do GitHub em V8 isolado com DOM simulado e harness de assertions; não é execução nativa de node --test, Chromium ou QA integral. As cinco PRs funcionais continuam em draft; nenhum run de Actions ou status externo foi encontrado nos HEADs consultados antes desta alteração. A última bateria integral validada permanece 395. Interface dos agentes e modelos inalterados.

## Décima entrega — CodeRabbit OSS e correção de revisão (24/11/2026)
- CodeRabbit confirmado em funcionamento no fork público: revisão da PR #67 no HEAD f0eee616dc04f75faed4525c6c98f0eeae10538f, comentário https://github.com/croquete1/ApolloOffice/pull/67#issuecomment-7131097387. Risco de merge mínimo, risco de arquitetura baixo; identificou recarregamento por HTTP 401 na atualização de logs que podia perder edições não guardadas.
- #67 ampliada (draft, HEAD 090dcdb4817ad2f8f9c500efc13b371288850131): só a leitura de logs opta por não recarregar após 401; mantém recuperação de autenticação preexistente nas restantes leituras e mostra instrução explícita para guardar uma cópia das alterações. RED isolado reproduziu o reload; GREEN e cinco testes de regressão PASS em V8 com DOM/transporte simulados. Não equivale a Node nativo, Chromium ou CI. Nova revisão solicitada no comentário 7131333599; aguarda confirmação do novo HEAD.
- Política versionada .coderabbit.yaml: revisão assertiva, pedidos manuais para controlar quota, sem revisão incremental automática, drafts elegíveis, sugestões consultivas (sem request-changes do bot) e instruções por caminho. Não substitui os gates existentes. Configuração efetiva no serviço e faturação por consumo desativada ainda por confirmar; nenhuma funcionalidade paga autorizada.
- F17 acrescentada ao roadmap para integração de resultados de revisão externos. O Watcher atual filtra bots; não atribuir privilégios de mantenedor ao CodeRabbit, nem executar automaticamente instruções em comentários. Nenhuma alteração funcional em main ou ativação Qwen; representação dos agentes preservada.

## Décima primeira entrega — evidência CodeRabbit no Steward (24/11/2026)
- A segunda revisão CodeRabbit da #67 confirmou cobertura do HEAD 090dcdb4817ad2f8f9c500efc13b371288850131, risco mínimo de integração e arquitetura, sem defeitos impeditivos concretos. Continua pendente QA nativo/browser. A configuração da #76 foi reconhecida pelo serviço (perfil ASSERTIVE; revisão automática desativada). O serviço reporta plano Advanced, não prova faturação zero; não foram solicitadas novas revisões nesta entrega.
- PR #77 (draft, HEAD 2ef565e20b568c9efa24e4b7a6057c041431201f): consulta explícita no painel Steward via endpoint autenticado, apenas repositórios ativos e confirmados públicos. Leitura paginada de comentários GitHub, validação da identidade numérica/login/tipo do bot e confirmação da PR/HEAD antes/depois da leitura. Estados consultivos: evidência atual, stale, não observada, desconhecida ou indisponível. Sem envio de dados a novos fornecedores, pedidos automáticos de revisão, execução de comentários, alterações de permissões ou merge.
- QA #77: quatro cenários JavaScript isolados PASS no handler de produção com DOM simulado (estado consultivo, HEAD incompatível, estado arbitrário, pedido simultâneo/painel substituído); testes Node correspondentes e quatro testes Rust adicionados, não executados nativamente. Marcadores HTML do CodeRabbit são formato observado não contratual; eventual alteração mantém cobertura desconhecida. Persistência/deduplicação de observações, ingestão de findings com Guard e recuperação end-to-end permanecem pendentes. Sem alteração do canvas ou modelos.

## Décima segunda entrega — persistência de evidência CodeRabbit (24/11/2026)
- PR #77 ampliada (draft, HEAD 35cf999b5d112830b9065aad42629480ddee1cda): persistência de uma observação histórica por repo/PR na tabela KV existente, sem armazenar corpos de comentários. Tickets de geração atribuídos atomicamente; resposta de consulta antiga não substitui a mais recente. Falhas conservam última observação bem-sucedida. Escritas restritas a repos ativos; consulta manual exige PR e HEAD presentes no snapshot Steward. Histórico apresentado com data e indicação de HEAD, expressamente não revalidado. Sem uso em gates, sem triggers CodeRabbit ou execução de findings.
- Reconhecimento consultivo de revisão em processamento e revisão não executada, mesmo quando a mensagem conserva marcadores antigos de cobertura. Formato observado no serviço, não uma API contratual. CodeRabbit estava a processar a #77 durante a entrega; não há ainda confirmação de cobertura do HEAD final. A configuração Assertive da #76 é reconhecida, mas ainda não integrada na main; não foram emitidos pedidos de revisão adicionais.
- QA: seis cenários SQLite isolados PASS usando os SQL exatos (rejeição de escrita stale, conservação do histórico em falha, persistência após reabertura, separação de repo, bloqueio de repo desativado, oito ligações concorrentes com tickets distintos). Três cenários JavaScript isolados PASS no handler (pending, skipped, current consultivo). Dois novos testes Rust e um teste Node escritos, sem execução nativa; não equivale a QA integral.
- Limites: um registo por PR consultada, sem retenção/limpeza automática ou histórico completo de versões; observação de comentários não prova aprovação, estado de CI ou conclusão da revisão. Persistência de findings, Guard/Watcher, recuperação, revisão de concorrência e suite integral pendentes. Interface dos agentes e modelos inalterados.

## Balanço por fase
| Fase | Estado | Falta |
|---|---|---|
| F01 | Parcial (#49) | Validação Docker/fnm em workers reais, logs upstream, ciclo de vida sem interrupções |
| F02 | Parcial (#48/#61) | Inbox/outbox duráveis, isolamento de falhas nas PRs abertas, deduplicação por PR/HEAD/review e recuperação end-to-end |
| F03 | Parcial (#48/#52/#63) | Unificar revisão/rebase/recheck e preservar autorizações; testes concorrentes reais |
| F04 | Parcial (#51/#57/#62) | Converter restantes decisões críticas; SQLITE_BUSY, disco cheio, recuperação e limites por repo |
| F05 | Parcial (#54/#62) | Retoma de chamadas de fornecedores/quotas e fases sem checkpoint seguro, CI após rebase e recuperação end-to-end |
| F06 | Parcial (#53/#58) | Cooldowns persistentes de todos os fornecedores, circuit breakers, fallback por papel |
| F07 | Parcial (#53/#56) | Previsão/reserva concorrente por prioridade e otimizações; limites durante execução |
| F08 | Parcial (#58) | Adaptar outros papéis, custo/complexidade calibrados, validação com providers reais |
| F09 | Parcial (#51/#52/#63) | Sync incremental, reconciliação sob mutações e validação de múltiplas páginas reais; limite conservador de ficheiros mantido |
| F10 | Parcial (#54/#56/#62) | Aging, causa comum e agrupamento, prevenção de sobreposições, validação remota de dependências |
| F11 | Parcial (#56) | GPU/VRAM, CI dinâmico, prioridade processos e validação sob pressão real |
| F12 | Parcial (#61/#62) | Isolamento efetivo por confiança/repo, segredos remotos, checkouts/caches executáveis e testes Docker/SSH reais |
| F13 | Parcial (#59) | Medir eficácia real e validar fluxos interativos e expiração após reinício |
| F14 | Implementação base integrada (#56) | Teste de persistência/reinício, perfis em máquinas reais e limites de CI |
| F15 | Parcial (#56/#58/#59) | Próxima tarefa elegível, diagnóstico central de todos os bloqueios, histórico de etapas |
| F16 | Parcial (#58/#59) | Tempo por fase e custo/merge com dados completos, baseline comparável, QA real Docker/Windows/providers, release assinada |
| F17 | Parcial (#67, #76, #77 draft) | Validar política/conta, QA consulta e persistência, ingestão de findings, Guard e integração segura |

Nenhuma fase completa é declarada encerrada sem cumprir todos os critérios abaixo. A última bateria automatizada tem 395 aprovações, não a soma de execuções repetidas.

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

## Sequência
Integrar PRs apenas após revisão e testes, uma fase de cada vez. Não ativar Qwen. Não alterar main ou instalação em produção sem evidência. Atualizar este ficheiro com PR, SHA e resultado dos testes de cada fase.
