//! Outbound adapters: the HTTP client implementing
//! `application::ports::RemoteGateway`, and the factory building one per
//! `{{product}}Instance`.

pub mod gateway_factory;
pub mod http_gateway;

pub use gateway_factory::HttpGatewayFactory;
pub use http_gateway::HttpGateway;
