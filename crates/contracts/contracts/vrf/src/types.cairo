use stark_vrf::Point;
use starknet::ContractAddress;

#[derive(Drop, Copy, Clone, Serde)]
pub enum Source {
    Nonce: ContractAddress,
    Salt: felt252,
}


#[derive(Drop, Copy, Clone, Serde, starknet::Store)]
pub struct PublicKey {
    pub x: felt252,
    pub y: felt252,
}

impl PublicKeyIntoPoint of Into<PublicKey, Point> {
    fn into(self: PublicKey) -> Point {
        Point { x: self.x, y: self.y }
    }
}
