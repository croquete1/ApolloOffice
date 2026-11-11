# Fiabilidade dos Hunters e preparação dos repositórios

Esta implementação limita-se a seis objetivos: interromper HUNT:DROP consecutivos, diagnosticar as falhas sem SQLite manual, verificar o ambiente, priorizar correções, explicar inatividade e reduzir downloads repetidos.

## Proteção de Hunters (P0)

- Três falhas consecutivas de **preflight** ou **setup**, no mesmo repositório e em jobs `hunt`, ativam uma suspensão de 30 minutos para novas pesquisas desse repositório.
- A suspensão não elimina jobs, não impede correções `fix` e não desativa outros repositórios.
- Falhas de transporte do worker durante o preflight ou setup seguem recuperação de máquina `X::Host` em vez de serem contabilizadas como falhas determinísticas do repositório. Se se esgotarem as tentativas da máquina, permanece diagnóstico na etapa `host`, sem avançar o contador do circuit breaker.
- Um setup bem-sucedido de um novo `hunt` limpa o contador. Após o prazo, a pesquisa pode tentar novamente; se voltar a falhar, o período é renovado.
- Em **Repositórios → Diagnóstico de execução**, o operador pode repor explicitamente o bloqueio **depois de corrigir o ambiente**. Este botão não força a execução de trabalho nem ignora os requisitos de autenticação.
- A opção original `hunt for bugs itself` continua a controlar se o repositório pode ser pesquisado.

## Diagnóstico de erros (P0)

- `GET /api/failures` devolve até 80 falhas reais de execução (`drop`), com identificador, repositório, fase, código de saída e razão; exige login. Exclui cancelamentos explícitos pelo operador (`dropped by the owner`) e remoções de repositórios (`repository removed`), sem esconder falhas anteriores que não tenham diagnóstico estruturado.
- `GET /api/job?id=N` inclui `failure` com fase, código de saída, máquina, data e cauda detalhada, limitada a 8 KiB.
- O backend oculta padrões comuns de credenciais antes de persistir a cauda, incluindo atribuições `DOCKER_AUTH_CONFIG` com JSON de autenticação e os campos `auth`/`identitytoken`. Isto **não substitui** o cuidado na produção de logs nem uma auditoria específica dos segredos em formatos imprevistos.
- O diálogo de cada trabalho mostra os detalhes. O painel de repositórios apresenta o histórico sem exigir comandos SQLite.
- Não foram criados endpoints sem autenticação.

## Preflight e filas (P1)

- Antes de clonar um repositório, o worker verifica a existência das ferramentas conhecidas identificadas no início do comando de setup: `npm`, `pnpm`, `yarn`, `cargo`, `go`, `python3` e `pip3`. Para os gestores JavaScript, verifica também `node`.
- O preflight é conservador: não executa scripts arbitrários nem substitui o setup verdadeiro; comandos personalizados continuam sujeitos à preparação completa.
- Correções elegíveis têm prioridade sobre novas pesquisas `hunt`, mantendo preferência por trabalhos de submissão e de retoma.
- Quando existem correções pendentes, o gerador de Hunters reserva capacidade para as mesmas. Com uma única lane e sem correções, a pesquisa continua disponível.
- As prioridades, a política de quotas de PR e o controlo de permissões existentes mantêm-se em vigor.

## Estados reais dos agentes (P1)

- A API de estado publica `w[role].idle_reason` e `ops.waiting`, `ops.paused`, `ops.blocked_hunts`. O valor `ops.waiting` exclui os IDs admitidos, incluindo trabalhos ainda com estado `ready` ou `submit`.
- O painel distingue pausas, tarefa não atribuída, existência de fila e pesquisas suspensas.
- Os textos são informativos: não afirmam que exista execução quando só há trabalhos pendentes; não apresentam estimativas inventadas.

## Preparação mais económica (P2)

- O worker Docker mantém agora um volume novo `apollo-office-npm-downloads` em `/var/cache/apollo-npm` **exclusivamente para a cache de tarballs `_cacache`**. O volume antigo `apollo-office-npm` deixa de ser montado e não é apagado automaticamente. Esta migração evita reutilizar instalações executáveis `_npx` anteriores.
- Cada comando executado pelo worker Docker recebe uma diretoria npm temporária exclusiva (`mktemp`, modo restrito), com ligação apenas à cache de downloads. As árvores executáveis `_npx`, logs e metadados não são reutilizados entre comandos nem sobrevivem normalmente ao fim da execução. Em timeout/falha, o wrapper conserva o tratamento de código de saída e remove a diretoria no seu `EXIT`; um processo terminado abruptamente pode deixar uma diretoria órfã, mas nunca será selecionada por outro comando.
- A manutenção periódica verifica, a cada ciclo do guard de disco, se a cache de downloads ultrapassa 4 GiB. Só executa `npm cache clean --force` quando o worker Docker está sem tarefas ativas. Um bloqueio transitório de admissão impede que uma nova tarefa arranque durante essa manutenção; o bloqueio é libertado também se a operação for cancelada. Nunca apaga `node_modules`, repositórios ou o volume nomeado. A cache poderá ser repopulada por futuras instalações, através do `npm ci`.
- O `npm ci` **não é omitido**: continua a aplicar o `package-lock.json` em cada setup, sem reutilizar cegamente uma árvore `node_modules` de tarefas anteriores.
- O volume soma-se a `apollo-office-work` e `apollo-office-cargo`; a alteração da configuração/identificador do worker na versão 0.10.4 deve ser aplicada apenas com o escritório pausado e sem jobs em execução. O armazenamento de código e os utilizadores dentro do mesmo contentor **não representam uma fronteira forte de isolamento entre repositórios potencialmente maliciosos**: para esse cenário é necessário um worker/contentor separado por nível de confiança.
- O benefício principal é reduzir descarregamentos e tráfego, não prometer que o custo total da instalação passa a zero.

## Validação antes de publicar

Na raiz de uma cópia limpa da branch:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
node --test quality/*.test.mjs
python3 -m unittest discover -s quality -p 'test_*.py'
python3 quality/browser_smoke.py
cargo build --release --locked
```

Com Docker, construir a nova imagem em tag temporária, verificar a cache declarada e testar falhas de build sem remover o contentor em produção. Para confirmar o circuito, simular três setups falhados no mesmo repositório, observar suspensão e recuperação após sucesso ou reset manual. Manter publicação assinada Ed25519 e auto-updates desligados por omissão. Não utilizar GitHub Actions.
