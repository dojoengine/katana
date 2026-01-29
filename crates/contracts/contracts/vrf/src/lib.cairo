pub mod vrf_provider {
    pub mod vrf_provider;
    pub mod vrf_provider_component;
    pub mod vrf_provider_upgrader;
}

pub mod vrf_consumer {
    pub mod vrf_consumer_component;
}

pub mod vrf_account {
    pub mod vrf_account;
    pub mod vrf_account_component;

    #[cfg(test)]
    pub mod tests {
        pub mod common;
        pub mod test_dice;
        pub mod test_upgrade;
    }
}

pub mod mocks {
    pub mod account_mock;
    pub mod vrf_consumer_mock;
}
pub use vrf_consumer::vrf_consumer_component::VrfConsumerComponent;

pub use vrf_provider::vrf_provider_component::{
    IVrfProvider, IVrfProviderDispatcher, IVrfProviderDispatcherTrait,
};


pub mod types;
pub use types::{PublicKey, Source};
// #[cfg(test)]
// pub mod tests {
//     pub mod common;
//     pub mod test_dice;
// }


