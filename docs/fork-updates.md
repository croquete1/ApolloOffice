# Atualizações do fork Apollo Office (CachyOS)

O executável personalizado é instalado em `~/.local/bin/apollo-office`, os dados em
`~/.local/share/apollo-office` e o serviço mantém o nome `apollo-office.service`.
**Nunca substituir pelo binário de Silvertree2010/ApolloOffice.**

## 1. Atualizador permanente local (intermédio)

O comando `apollo-office-update`, disponibilizado separadamente ao utilizador, instala-se
uma vez com `python3 -u ~/Transferências/apollo-office-update --configurar`
(ou `~/Downloads/apollo-office-update`). Em atualizações futuras:

- `~/.local/bin/apollo-office-update --verificar`: consulta a main, não instala.
- `~/.local/bin/apollo-office-update`: compila a main atual.

Antes de instalar, selecionar **Pause office** e deixar terminar trabalhos/merges.
A ferramenta exige o serviço, verifica o estado persistente, executa os gates locais,
faz cópias do binário e dados, substitui o executável com rename atómico e reverte
o binário se o novo serviço não arrancar. É **execução manual**, não um daemon.

O binário é compilado a partir do código autenticado no GitHub e **não** usa o
canal de updates assinados: são dois percursos separados, não duas fontes de atualização automática.

## 2. Distribuição assinada, independente de GitHub Actions

O atualizador integrado em `src/upd.rs` aceita **apenas** o canal explicitamente
configurado por URL HTTPS e chave pública Ed25519 do **nosso fork**. Verifica o
`latest.json.sig` sobre os bytes exatos de `latest.json`, depois confirma o
SHA-256 do executável e a versão superior à instalada.

No computador Linux x64 que publica a release:

1. Criar **uma única chave privada**, fora do repositório:

   ```bash
   mkdir -p ~/.config/apollo-office
   chmod 700 ~/.config/apollo-office
   openssl genpkey -algorithm ED25519 -out ~/.config/apollo-office/release-ed25519.pem
   chmod 600 ~/.config/apollo-office/release-ed25519.pem
   ```

2. Integrar e validar o código do fork na `main`, com `Cargo.toml` e
   `Cargo.lock` numa versão **nova** (a primeira preparada é `0.10.1`).
   Repetir o incremento de versão em cada release; apenas o SHA de Git não
   é suficiente para atualizar o executável integrado.

   A primeira release assinada `fork-v0.10.1` foi publicada em 11/11/2026.
   A versão `0.10.2` é uma release de validação do percurso completo de
   atualização nativa (0.10.1 → 0.10.2); a mudança de versão, por si só,
   não implica novas funcionalidades. Não publicar sem passar os gates.

3. Na raiz da `main` limpa, autenticado com `gh auth login`, executar:

   ```bash
   export APOLLO_SIGNING_KEY_FILE="$HOME/.config/apollo-office/release-ed25519.pem"
   bash scripts/publish-signed-linux.sh
   ```

O script executa testes, compila em Release, cria o manifesto, assina localmente e
publica os três ficheiros (`apollo-office-linux-x64`, `latest.json`,
`latest.json.sig`) numa GitHub Release com tag `fork-vX.Y.Z`.
A tag `fork-v*` **não corresponde** ao trigger upstream `v*` e não requer
novos GitHub Actions. A chave privada **nunca** é enviada ao GitHub.

### Configurar o serviço para o canal assinado

Copiar do resultado do script a chave pública de 64 caracteres hexadecimais.
**Não usar a chave de Silvertree2010/ApolloOffice.**

Criar um *drop-in* systemd do utilizador para o serviço existente:

```ini
[Service]
Environment="APOLLO_OFFICE_UPDATE_URL=https://github.com/croquete1/ApolloOffice/releases/latest/download"
Environment="APOLLO_OFFICE_UPDATE_PUBKEY=CHAVE_PUBLICA_HEXADECIMAL_DO_FORK"
```

Colocar o conteúdo em
`~/.config/systemd/user/apollo-office.service.d/updates.conf` para um serviço
systemd --user, fazer `systemctl --user daemon-reload` e reiniciar apenas
quando não existirem trabalhos ativos. Se a unidade for de sistema, usar o
respetivo administrador/systemd; não alterar a unidade errada.

### Definições → Sistema → Atualizações

- **Verificar atualizações**: consulta apenas o canal assinado.
- **Atualizar agora**: só é autorizado depois de selecionar **Pause office**
  e quando não existirem trabalhos, locks ou merges ativos. Uma condição
  de corrida interrompe a instalação.
- O modo automático permanece **desativado por omissão**; só pode ser ativado
  se URL e chave pública de confiança estiverem configuradas.

O download possui limites de tamanho. O executável anterior é preservado
com extensão `.old` no Unix para recuperação. Se o novo binário não arrancar
mesmo depois do `exec`, a recuperação manual continua disponível: parar o
serviço, restaurar a cópia `.old`, iniciar o serviço e investigar.

## 3. Estado de validação

A versão `fork-v0.10.1` foi publicada e assinada localmente em 11/11/2026,
com os três assets Linux x64 verificados e instalação do código da `main`
confirmada no CachyOS. A atualização nativa pelo botão ainda necessita do
teste end-to-end com a versão superior `0.10.2`, seguindo
[o roteiro de validação](first-signed-upgrade-test.md).

A versão `0.10.2` só pode ser publicada depois de integrar a respetiva PR
com validação de Rustfmt, Clippy, Rust, Node, Python, Chromium e Release.
As atualizações automáticas permanecem desligadas até o teste terminar.
