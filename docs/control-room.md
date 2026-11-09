# Apollo Control Room — interface animada

Esta versão substitui apenas a apresentação do painel. O Apollo Office mantém os mesmos endpoints, agentes, mecanismos de autenticação, aprovação de PRs e operações.

## O que é apresentado

- Dashboard com dados efetivos de `/api/state` e `/api/ws`, recebidos através de `AO.subscribeState()` sem substituir o callback original; cada subscritor é isolado contra erros.
- Os painéis de tarefas e PRs aparecem antes do escritório virtual na ordem visual e na navegação por teclado. O escritório, embora visível e animado, é uma área secundária do dashboard.
- Dez postos de trabalho, um por agente original, com personagens vetoriais não pixelizadas.
- Animações de respiração, pestanejar e digitação conforme os estados de atividade.
- Movimentação temporária de uma personagem entre secretárias apenas quando uma mesma tarefa muda de fase entre dois agentes identificáveis (por exemplo, `fix` → `ci`). Não há deslocações aleatórias nem tarefas simuladas.
- Animações distintas nos ecrãs de CI, Reviewer, Repro e Submit, ligadas ao estado de trabalho.
- Expressões e indicadores de último resultado (sucesso, problema ou erro) a partir de `w.mood`, sem classificar resultados por texto livre.
- Breve reação do monitor a um novo comando real em `w.cmds`; diálogo de agente com os últimos quatro comandos disponíveis.
- Celebração apenas quando uma tarefa anteriormente associada a um agente surge como concluída (`dn[].st === "done"`).
- Painéis de tarefas, PRs, eventos, integrações dos últimos 30 dias e informações de máquinas, sem estimativas fictícias de progresso ou GPU.
- Pipeline informativo de dez fases reais, sem agrupar validação ou submissão automática com aprovação humana: Pendente, Triagem, Pesquisa, Reprodução, Correção, Testes, Revisão, Validação, Aprovação e Submissão. Todas as contagens vêm da base de dados. Não contém botões de ações rápidas.
- Os indicadores de tarefas em aberto e concluídas usam o mapa completo `s.n`, não a lista de pré-visualização limitada. A lista de PRs continua limitada aos últimos 40 registos.
- As personagens iniciam-se num estado desconhecido, sem simular disponibilidade antes da primeira resposta do servidor.
- O estado de cada agente vem exclusivamente de `s.w[role].st`; o backend já representa as tarefas persistentes nesse campo. Uma tarefa na fila não constitui prova de que determinado agente a está a executar.
- As celebrações exigem um identificador de tarefa previamente observado e a respetiva conclusão recente, dentro de 60 segundos da hora do servidor.
- Limites da API respeitados nas listas de pré-visualização: máximo de 60 tarefas em aberto, 40 PRs recentes e 25 trabalhos terminados recentes. Os totais de trabalhos vêm de `s.n`, não dessas listas. Uma máquina só apresenta medições atuais quando existe telemetria com menos de 120 segundos; em caso contrário mostra a hora da última leitura ou um aviso de indisponibilidade.
- Filtro por repositórios que aparecem nos registos de tarefas/PRs; não representa necessariamente todos os repositórios configurados.
- Redução de movimento via preferência do sistema ou interruptor visível. As animações temporárias de celebração e de novos comandos são limpas após a execução, em caso de suspensão e quando os dados ficam desatualizados.
- A lista de agentes apresenta agora atividade real (`w.say`) e modelo atribuído (`w.md`), incluindo modelos locais Ollama como o Qwen 3 8B.
- O cabeçalho indica a última atualização do servidor (`s.t`); quando a ligação expira, os estados passam a «Por confirmar» e as métricas de trabalho ativo deixam de ser apresentadas como atuais.
- Navegação entre postos por setas: cinco colunas em ecrãs amplos e duas colunas até 530 px. Os botões continuam acessíveis por Tab.
- Foco preservado durante atualizações de tarefas e PRs: mantém-se no elemento equivalente; se este desapareceu, transfere-se para a respetiva secção.
- A informação das máquinas distingue explicitamente telemetria recente, última leitura histórica e situação por confirmar, incluindo quando o canal de estado deixa de atualizar.

## Máquina e telemetria — revisão PT-PT

O painel **Máquinas e recursos** apresenta cartões com as leituras disponibilizadas
pelo backend, sem chamadas adicionais ao sistema operativo e sem introduzir
gráficos ou personagens da interface original:

- **Processador:** número de núcleos e carga média reportados. A carga média
  **não é apresentada como percentagem de utilização instantânea da CPU**.
- **Memória e disco:** usados/livres com barras percentuais, apenas quando
  os valores são numéricos e consistentes.
- **Temperatura:** valor do sensor reportado, sem presumir que se trata da
  CPU ou GPU. O backend não identifica qual o sensor.
- **Atividade:** número de tarefas atribuídas que constam da lista recebida.
  Essa lista é uma pré-visualização e **não prova a ocupação total dos postos**.
- **Validade dos dados:** telemetria com menos de 120 segundos, host ativo,
  ligação não declarada indisponível e estado do servidor recente. Fora desses
  critérios são mostrados apenas o estado e a última leitura, **sem representar
  métricas antigas como atuais**.

A UI personalizada foi preservada. O projeto usa os ficheiros
`ui/control-room.js`, `ui/control-room.css`, `ui/office-v2.js` e
`ui/office-v2.css`; a interface pixelizada permanece apenas como código de
compatibilidade. Uma captura da sala pixelizada não prova que o fork original
tenha sido instalado: pode indicar um binário antigo ou que o Control Room
não foi carregado.

Para verificar **a instância que está realmente aberta** no browser,
abra a consola de desenvolvimento e execute:

```js
({
  personalizado: Boolean(document.getElementById("aoControlRoom")),
  versaoModernaAtiva: window.AO?.modernRoom === true,
  fallback: document.body.classList.contains("ao-fallback-no-pixel")
})
```

Um resultado com `personalizado: false` exige verificar o executável em
execução e o carregamento de `control-room.js`, sem instalar uma release
oficial por cima do fork. A aplicação incorpora os ficheiros `ui/` no binário:
uma mudança de código remoto **não atualiza** automaticamente a UI local.

## Fontes e limitações

Os movimentos mostram os estados que o Apollo Office disponibiliza, não o conteúdo interno do raciocínio ou dos modelos. As personagens são ilustrações 2.5D construídas com SVG e CSS, e não modelos 3D com animação esquelética. O nome do modelo apresentado em cada agente vem do respetivo estado; a interface não altera a configuração do Qwen/Ollama.

**Esta versão não inclui** circulação livre sem tarefas associadas, sincronização labial nem uma cena 3D navegável. Os movimentos entre postos são representações de transições reais do fluxo, implementadas com Web Animations API. Uma mudança detetada após mais de 10 segundos sem observações não gera uma animação de passagem de trabalho. Melhorias adicionais dependem da validação funcional e do desempenho no navegador.

Se o renderizador moderno falhar ao processar um estado, a interface original é restaurada automaticamente. O desenho pixelizado antigo permanece no código para compatibilidade, mas não é apresentado, nem durante a recuperação após falha. O fallback remove as subscrições, os handlers globais de teclado/visibilidade, animações temporárias e timers do Control Room. Restaura os formulários e controlos operacionais, mas mantém o canvas pixelizado desativado.

## Verificação

```bash
node --test
python3 quality/browser_smoke.py
cargo test --locked
cargo run --release
```

A verificação de navegador utiliza Chrome ou Chromium em modo headless e um servidor HTTP temporário restrito a `127.0.0.1`. Serve os ficheiros reais de `ui/` e fornece dois estados sintéticos (agentes em repouso e Fixer ativo com `ollama:qwen3:8b`). **Não** contacta um backend de produção, não executa agentes, não cria PRs e não necessita de credenciais. A CI já existente corre os testes Node e Rust em Ubuntu/Windows e a verificação de navegador em Ubuntu.

Aceder à ligação de autenticação impressa pela aplicação e testar:

1. Estados work/idle/sleep/paused, incluindo pausa global.
2. Abertura de tarefa e configuração do modelo ao selecionar um agente.
3. Atualização por WebSocket e alternativa por polling.
4. Interrupção da ligação por mais de 12 segundos (aviso de dados desatualizados).
5. Filtro por repositório, navegação por teclado, Escape e foco da janela de detalhes.
6. Interruptor de animações, `prefers-reduced-motion` e ecrãs pequenos.
7. Regressão das ações existentes de aprovação, configurações, custos, repositórios e máquinas.

**Nota:** as verificações em JavaScript isolado não substituem a execução desta suíte no Node, a validação de browser, os testes de integração Rust nem a medição de desempenho local. A criação da branch não atualiza automaticamente um binário instalado.

## Studio 2.5D — revisão visual premium

O escritório virtual utiliza agora `ui/office-v2.js` e `ui/office-v2.css`, carregados antes do Control Room para preservar os componentes operacionais existentes. Mantém-se como **área secundária** do dashboard; não são acrescentadas ações rápidas.

- Dez personagens SVG individualizadas, com cabelo, tons de pele, roupa, expressões, acessórios profissionais e juntas de braços/pernas próprias. Não utiliza pixel art, imagens externas nem motor 3D.
- Sala com arquitetura em camadas, janelas e horizonte, iluminação indireta, vegetação, mobiliário, tapete e chão em perspetiva. Toda a ilustração é vetorial e adaptável.
- Estados **reais** alimentados por `s.w[role].st`: trabalho (digitação ou gesto do papel), repouso (respiração/pestanejar), pausa ou ausência de estado. A passagem entre trabalho e repouso aciona uma reação curta e não um ciclo falso de atividade.
- Handoff apenas quando a **mesma tarefa** muda realmente de agente; a personagem percorre o corredor central, faz uma breve paragem no posto de destino e regressa. A reação do colega acompanha exclusivamente esse evento. Nenhuma ação é enviada aos agentes.
- Em ecrãs largos, cinco postos por fila; em telemóvel, dois postos por fila. Os controlos de acessibilidade e as preferências de movimento reduzido continuam ativos.
- `quality/office-v2.test.mjs` cobre a anatomia SVG, unicidade, decoração idempotente, transições reais, movimento reduzido e integração. `quality/browser_smoke.py` valida também a nova sala e a suspensão das animações em Chromium.

A implementação continua **2.5D**, conforme a opção leve para execução simultânea com modelos locais. Não inclui modelos esqueléticos tridimensionais, sincronização labial ou circulação sem tarefa associada. Nenhuma configuração do Ollama ou do Qwen foi alterada. O impacto de desempenho no computador do utilizador continua a exigir medição local.
