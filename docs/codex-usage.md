# Limites do Codex no Apollo Office

O escritório pixelizado apresenta os limites **de 5 horas** e **semanais** associados à sessão local do Codex. A vista moderna opcional apresenta os mesmos valores.

## Fonte dos dados

A aplicação utiliza o método oficial `account/rateLimits/read` do Codex App Server, através de `codex app-server`. A consulta é efetuada **localmente** pelo mesmo utilizador que executa o serviço Apollo Office.

- Cada janela mostra a **percentagem restante**: `100 - usedPercent`.
- A reposição é calculada a partir de `resetsAt`, fornecido pelo Codex, nunca estimado pelos tokens consumidos.
- São reconhecidas janelas com duração de **300 minutos** e **10 080 minutos**; `primary` não é automaticamente interpretada como 5 horas.
- Quando o serviço não disponibiliza uma janela, aparece **Sem dados**. Isto pode depender do plano e dos limites em vigor.
- O painel consulta o servidor aproximadamente a cada **2 minutos**, e a contagem decrescente avança localmente a cada segundo. O backend aplica uma cache de dois minutos e timeout de 10 segundos.

## Requisitos

1. Codex CLI instalado e autenticado através de uma **conta ChatGPT** no mesmo utilizador do serviço, por exemplo com `codex login`.
2. O serviço Apollo Office tem de conseguir encontrar o comando `codex` no seu `PATH`. Se necessário, pode indicar a localização do executável na variável de ambiente `APOLLO_OFFICE_CODEX` do serviço.
3. As sessões autenticadas apenas por chave API podem não disponibilizar limites ChatGPT através deste método.

**Não é necessário copiar tokens, introduzir palavras-passe no Apollo Office ou aceder a `~/.codex/auth.json`.** A leitura devolve apenas percentagens, horários de reposição e estado operacional. O endpoint `GET /api/codex/limits` requer uma sessão autenticada no Apollo Office.

Se surgir **Indisponível** ou **Sem ligação**, confirme a sessão do Codex CLI, o `PATH` do serviço e a ligação à Internet. Os valores da última consulta não são mantidos na interface como se fossem atuais após uma falha.

Referência oficial: [Codex App Server — Account rate limits](https://learn.chatgpt.com/docs/app-server).
