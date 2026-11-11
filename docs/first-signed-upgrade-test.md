# Validação do primeiro upgrade assinado do fork (0.10.1 → 0.10.2)

Este procedimento valida o mecanismo nativo de atualização, sem instalar
releases de `Silvertree2010/ApolloOffice` e sem ativar atualizações automáticas.

## Pré-requisitos

- A `main` já contém a PR que eleva `Cargo.toml` e `Cargo.lock` a **0.10.2**,
  depois de Rustfmt, Clippy, Rust, Node, Python e Chromium terem passado.
- A release `fork-v0.10.1` está publicada e foi assinada pela chave privada
  local `~/.config/apollo-office/release-ed25519.pem`.
- O drop-in `~/.config/systemd/user/apollo-office.service.d/updates.conf`
  já contém a URL do **nosso fork** e a **chave pública** correspondente.
- O serviço `apollo-office.service` executa o binário personalizado em
  `~/.local/bin/apollo-office`. Não publicar novas chaves: a chave pública
  já instalada é o identificador de confiança do canal.
- Confirmar na interface, em **Definições → System → This installation → Version**,
  que a versão em execução é exatamente **0.10.1**. Pode comparar com
  `~/.local/bin/apollo-office status`. Se for diferente, **não publicar** a
  nova release: o cenário 0.10.1 → 0.10.2 deixará de poder ser validado.
- Antes da publicação, em **Definições → System → Atualizações**, desligar
  as **atualizações automáticas**, gravar e voltar a abrir a página
  para confirmar. O watcher automático não pode instalar a 0.10.2
  antes de se experimentar o botão **Atualizar agora**.

## 1. Confirmar o serviço sem o parar

```fish
systemctl --user is-active apollo-office.service
curl -sS -o /dev/null -w 'HTTP %{http_code}\n' http://127.0.0.1:47300/
systemctl --user cat apollo-office.service
~/.local/bin/apollo-office status
```

O serviço deve estar `active`, o HTTP deve responder (200 ou redirecionamento
para autenticação) e a versão apresentada na interface deve ser **0.10.1**.
Não partilhar logs com segredos nem o conteúdo da chave privada.

**Bloqueio obrigatório:** não avançar para a publicação sem confirmar a
versão **0.10.1** e sem guardar a opção de atualizações automáticas
**desligada** na interface.

## 2. Publicar a versão 0.10.2 após merge e testes

Numa **cópia limpa** da `main` no CachyOS, após o merge e a revisão,
copiar para `sha_aprovado` o **SHA exato do commit de squash aprovado**.
Não usar a versão ou o HEAD remoto como substituto desse compromisso:
a `main` pode mudar entre a revisão e a publicação.

```fish
set sha_aprovado SUBSTITUIR_PELO_SHA_DE_MERGE_APROVADO
string match -rq '^[0-9a-f]{40}$' -- "$sha_aprovado"
or exit 1

set workdir (mktemp -d)
git clone --depth 1 --branch main https://github.com/croquete1/ApolloOffice.git "$workdir/ApolloOffice"
or exit 1
cd "$workdir/ApolloOffice"
or exit 1
test (git rev-parse HEAD) = "$sha_aprovado"
or exit 1
grep '^version = "0.10.2"$' Cargo.toml
or exit 1
set -lx APOLLO_SIGNING_KEY_FILE "$HOME/.config/apollo-office/release-ed25519.pem"
bash scripts/publish-signed-linux.sh
```

Se o SHA não coincidir, parar e revalidar a `main`. O publicador
volta a executar todos os gates, fixa o SHA da `main`, compila
`release`, assina `latest.json` em Ed25519, verifica a assinatura localmente
e publica `fork-v0.10.2` sem GitHub Actions. Confirmar no terminal
`RELEASE_PUBLICADA=fork-v0.10.2`. Não voltar a executar se a tag já existir.

## 3. Testar exclusivamente pelo interface

1. Abrir `http://127.0.0.1:47300/`, autenticar-se e aceder a
   **Definições → Sistema → Atualizações**.
2. Manter as **atualizações automáticas desligadas**.
3. Selecionar **Verificar atualizações**: deve ser mostrada a release
   assinada `0.10.2` (nunca uma release upstream).
4. Confirmar que não há jobs, merges, Vet ou outros trabalhos ativos.
5. Selecionar **Pause office** e só então **Atualizar agora**.
6. Aguardar o reinício do serviço e confirmar que a aplicação voltou.
7. Verificar versão e serviço:

```fish
systemctl --user is-active apollo-office.service
~/.local/bin/apollo-office status
curl -sS -o /dev/null -w 'HTTP %{http_code}\n' http://127.0.0.1:47300/
```

A versão deverá ser `0.10.2`; depois de nova verificação, a interface
não deverá oferecer a mesma release novamente. Só retirar **Pause office**
depois de confirmar o funcionamento. O binário anterior fica disponível
em `~/.local/bin/apollo-office.old`.

## 4. Critérios de aprovação e recuperação

- **PASS**: release `fork-v0.10.2` assinada com a mesma chave, versão
  `0.10.2` detetada e instalada pelo botão, sem jobs interrompidos,
  serviço novamente ativo, UI acessível e nova verificação sem update.
- **FAIL**: assinatura inválida, versão não detetada, falta de pausa,
  operação durante jobs, ausência de reinício ou falha de arranque.
  Neste caso não ativar atualizações automáticas; preservar os logs
  com dados sensíveis removidos e a cópia `.old`. Usar o
  atualizador manual de `main` apenas depois de analisar o problema.

Fazer uma **cópia de segurança encriptada fora do computador** da chave
Ed25519 privada, com permissões de acesso restritas. Nunca a colocar
na pasta do repositório, em GitHub Issues, em Notion ou no chat.
