# Apollo Control Room — interface animada

Esta versão substitui apenas a apresentação do painel. O Apollo Office mantém os mesmos endpoints, agentes, mecanismos de autenticação, aprovação de PRs e operações.

## O que é apresentado

- Dashboard com dados efetivos de `/api/state` e `/api/ws`, reaproveitando `AO.onState`.
- Dez postos de trabalho, um por agente original, com personagens vetoriais não pixelizadas.
- Animações de respiração, pestanejar e digitação conforme os estados de atividade.
- Celebração apenas quando uma tarefa anteriormente associada a um agente surge como concluída (`dn[].st === "done"`).
- Painéis de tarefas, PRs, eventos, integrações dos últimos 30 dias e informações de máquinas, sem estimativas fictícias de progresso ou GPU.
- Filtro por repositórios que aparecem nos registos de tarefas/PRs; não representa necessariamente todos os repositórios configurados.
- Redução de movimento via preferência do sistema ou interruptor visível.

## Fontes e limitações

Os movimentos mostram os estados que o Apollo Office disponibiliza, não o conteúdo interno do raciocínio ou dos modelos. As personagens são ilustrações 2.5D construídas com SVG e CSS, e não modelos 3D com animação esquelética. O nome do modelo apresentado em cada agente vem do respetivo estado; a interface não altera a configuração do Qwen/Ollama.

**Esta versão não inclui** circulação entre secretárias, sincronização labial nem uma cena 3D navegável. Estas melhorias devem ser avaliadas após validação da interface funcional e do custo de renderização no navegador.

Se o renderizador moderno falhar ao processar um estado, a interface original é restaurada automaticamente. O desenho pixelizado legado permanece nos ficheiros de origem para permitir a reversão.

## Verificação

```bash
node --test quality/control-room.test.mjs
cargo test
cargo run --release
```

Aceder à ligação de autenticação impressa pela aplicação e testar:

1. Estados work/idle/sleep/paused, incluindo pausa global.
2. Abertura de tarefa e configuração do modelo ao selecionar um agente.
3. Atualização por WebSocket e alternativa por polling.
4. Interrupção da ligação por mais de 12 segundos (aviso de dados desatualizados).
5. Filtro por repositório, navegação por teclado, Escape e foco da janela de detalhes.
6. Interruptor de animações, `prefers-reduced-motion` e ecrãs pequenos.
7. Regressão das ações existentes de aprovação, configurações, custos, repositórios e máquinas.

**Nota:** os testes de browser, integração Rust e medição de desempenho requerem execução local. A criação da branch não atualiza automaticamente um binário instalado.
