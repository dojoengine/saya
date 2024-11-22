use url::Url;

pub mod atlantic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProverIdentifier {
    AtlanticProver(String, Url),
    LocalProver,
}

impl ProverIdentifier {}
