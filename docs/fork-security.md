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

Sem as credenciais, a autenticação Google fica indisponível com uma mensagem explícita. O comando local `apollo-office login-link` continua a permitir iniciar sessão.

## Validação

Executar `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `node --test` e as suítes Python. Não utilizar a instalação de produção para testar alterações de autenticação ou de atualização.
