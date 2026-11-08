# Configuração segura do fork Apollo Office

Este repositório inclui personalizações que não fazem parte dos lançamentos oficiais. **Não instale lançamentos de `Silvertree2010/ApolloOffice` por cima desta versão**: poderia perder os elementos visuais e as correções do fork.

## Atualizações protegidas

As atualizações automáticas estão desligadas por defeito, incluindo nas instalações com a opção antiga gravada como ativa. As opções de atualização só funcionam após configurar **ambas** as variáveis:

```bash
export APOLLO_OFFICE_UPDATE_URL='https://exemplo.pt/releases/apollo-office/download'
export APOLLO_OFFICE_UPDATE_PUBKEY='<chave pública Ed25519 de 32 bytes em hexadecimal>'
```

A URL tem de utilizar HTTPS e não pode conter credenciais, query string nem fragmento. Os ficheiros publicados nessa URL devem conter um `latest.json` e respetiva assinatura `latest.json.sig` válida para a chave indicada. A assinatura cobre o manifesto, que contém o SHA-256 de cada binário. O programa associa o ficheiro preparado ao par **URL + chave pública**, rejeitando versões preparadas por outro canal, incluindo as herdadas da configuração anterior. A instalação continua a confirmar o hash do binário.

**Não reutilize a chave pública nem os ficheiros de assinatura do projeto oficial.** Esta configuração é opcional. Sem uma infraestrutura própria de releases assinadas, a forma suportada de atualizar é instalar manualmente uma compilação validada do fork, preservando a pasta de dados.

## Autenticação Google

O código não contém agora um segredo OAuth de cliente. Configure credenciais do **seu próprio cliente OAuth**, criado na Google Cloud Console, antes de ativar a ligação à conta:

```bash
export APOLLO_OFFICE_GOOGLE_ID='<ID público do seu cliente OAuth>'
export APOLLO_OFFICE_GOOGLE_SECRET='<segredo do seu cliente OAuth>'
```

A URI de redirecionamento registada no cliente Google deve corresponder a `http://127.0.0.1:<porto>/auth/google/cb`, utilizando o porto configurado no Apollo Office. PKCE, `state` de utilização única e validação da conta continuam ativos. O segredo é fornecido por variável de ambiente, não por ficheiro de código nem pela interface. Não inclua o segredo em comandos ou registos que possam ser publicados.

**A credencial anteriormente incorporada no histórico do Git deve ser considerada exposta e revogada/substituída no fornecedor.** A remoção do código atual não a elimina dos commits antigos. A rotação de credenciais requer acesso ao respetivo projeto Google Cloud, que esta alteração não pode substituir.

### Invalidar o segredo OAuth exposto

Se o cliente OAuth antigo pertencer ao projeto Google Cloud de terceiros (incluindo o projeto oficial), **só o titular desse projeto pode revogar o segredo**. Não tente reutilizar esse cliente: crie um cliente OAuth independente para o fork e contacte em privado o responsável pelo cliente antigo para solicitar a sua revogação.

Se for o titular do projeto afetado, aceda a [Google Auth Platform — Clients](https://console.cloud.google.com/auth/clients), escolha o projeto e o cliente OAuth antigo. Use **Add Secret** para criar um segredo novo, configure-o na instalação e confirme que a autenticação funciona; depois **Disable** e **Delete** no segredo antigo. Para revogação imediata em caso de exposição, a Google também documenta **Reset Secret**, com possível interrupção temporária da autenticação.

A chave antiga continua recuperável no histórico Git, mas deixa de ser utilizável depois de revogada no fornecedor. Não publique os novos valores no GitHub, na documentação, em logs ou nesta conversa. A documentação oficial está em [Manage OAuth Clients](https://support.google.com/cloud/answer/15549257?hl=en).

### Verificação da instalação personalizada

Este fork ainda não publica binários de release próprios. **Não utilize a release do projeto oficial para atualizar esta instalação.** Compile o código deste repositório com `cargo build --release --locked`, pare a instância em execução e preserve uma cópia da pasta de dados e do executável anterior antes de substituir qualquer ficheiro. No reinício, mantenha a mesma configuração de `--data` e as variáveis de ambiente OAuth aplicáveis. Se existir arranque automático, confirme que aponta para o executável personalizado.

Compilar o código no GitHub ou criar um PR **não atualiza o executável que já está instalado no computador**; essa operação exige acesso à máquina.

Sem as credenciais, a autenticação Google fica indisponível com uma mensagem explícita. O comando local `apollo-office login-link` continua a permitir iniciar sessão.

## Validação

Executar `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `node --test` e as suítes Python. Não utilizar a instalação de produção para testar alterações de autenticação ou de atualização.
