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

## 1. Confirmar o serviço sem o parar

```fish
systemctl --user is-active apollo-office.service
curl -sS -o /dev/null -w 'HTTP %{http_code}\n' http://127.0.0.1:47300/
systemctl --user cat apollo-office.service
```

O serviço deve estar `active` e o HTTP deve responder (200 ou redirecionamento
para autenticação). Não partilhar logs com segredos nem o conteúdo da chave privada.

## 2. Publicar a versão 0.10.2 após merge e testes

Numa **cópia limpa** da `main` no CachyOS (não na instalação executada):

```fish
set workdir (mktemp -d)
git clone --depth 1 --branch main https://github.com/croquete1/ApolloOffice.git "$workdir/ApolloOffice"
or exit 1
cd "$workdir/ApolloOffice"
or exit 1
grep '^version = "0.10.2"$' Cargo.toml
or exit 1
set -lx APOLLO_SIGNING_KEY_FILE "$HOME/.config/apollo-office/release-ed25519.pem"
bash scripts/publish-signed-linux.sh
```

O publicador volta a executar todos os gates, fixa o SHA da `main`, compila
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
