// Adapted from ipnetwork 0.20.0:
// https://github.com/achanda/ipnetwork
//
// Copyright 2020 Developers of the ipnetwork project
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::error::Error;
use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

const IPV4_BITS: u8 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    InvalidAddr(String),
    InvalidPrefix,
    InvalidCidrFormat(String),
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddr(addr) => write!(f, "invalid address: {addr}"),
            Self::InvalidPrefix => write!(f, "invalid prefix"),
            Self::InvalidCidrFormat(cidr) => write!(f, "invalid cidr format: {cidr}"),
        }
    }
}

impl Error for NetworkError {}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ipv4Network {
    addr: Ipv4Addr,
    prefix: u8,
}

impl Ipv4Network {
    pub const fn new(addr: Ipv4Addr, prefix: u8) -> Result<Self, NetworkError> {
        if prefix > IPV4_BITS {
            Err(NetworkError::InvalidPrefix)
        } else {
            Ok(Self { addr, prefix })
        }
    }

    pub const fn prefix(self) -> u8 {
        self.prefix
    }

    pub fn network(self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) & self.mask())
    }

    pub fn broadcast(self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) | !self.mask())
    }

    pub fn contains(self, ip: Ipv4Addr) -> bool {
        let mask = self.mask();
        u32::from(ip) & mask == u32::from(self.addr) & mask
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.contains(other.network()) || other.contains(self.network())
    }

    pub fn iter(self) -> AddressIterator {
        AddressIterator {
            next: Some(u32::from(self.network())),
            end: u32::from(self.broadcast()),
        }
    }

    const fn mask(self) -> u32 {
        if self.prefix == 0 {
            0
        } else {
            u32::MAX << (IPV4_BITS - self.prefix)
        }
    }
}

impl FromStr for Ipv4Network {
    type Err = NetworkError;

    fn from_str(cidr: &str) -> Result<Self, Self::Err> {
        let mut parts = cidr.split('/');
        let addr = parts
            .next()
            .ok_or_else(|| NetworkError::InvalidCidrFormat(cidr.to_owned()))?;
        let prefix = parts.next();
        if parts.next().is_some() {
            return Err(NetworkError::InvalidCidrFormat(cidr.to_owned()));
        }

        let addr = addr
            .parse()
            .map_err(|_| NetworkError::InvalidAddr(addr.to_owned()))?;
        let prefix = prefix.map_or(Ok(IPV4_BITS), |value| {
            value.parse::<Ipv4Addr>().map_or_else(
                |_| value.parse::<u8>().map_err(|_| NetworkError::InvalidPrefix),
                mask_to_prefix,
            )
        })?;
        Self::new(addr, prefix)
    }
}

fn mask_to_prefix(mask: Ipv4Addr) -> Result<u8, NetworkError> {
    let mask = u32::from(mask);
    let prefix = u8::try_from((!mask).leading_zeros()).map_err(|_| NetworkError::InvalidPrefix)?;
    if (u64::from(mask) << prefix) & u64::from(u32::MAX) == 0 {
        Ok(prefix)
    } else {
        Err(NetworkError::InvalidPrefix)
    }
}

#[derive(Clone, Debug)]
pub struct AddressIterator {
    next: Option<u32>,
    end: u32,
}

impl Iterator for AddressIterator {
    type Item = Ipv4Addr;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.next?;
        self.next = if next == self.end {
            None
        } else {
            Some(next + 1)
        };
        Some(next.into())
    }
}

impl IntoIterator for &Ipv4Network {
    type IntoIter = AddressIterator;
    type Item = Ipv4Addr;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_calculates_network_bounds() -> Result<(), NetworkError> {
        let network: Ipv4Network = "10.1.9.32/16".parse()?;
        assert_eq!(network.network(), Ipv4Addr::new(10, 1, 0, 0));
        assert_eq!(network.broadcast(), Ipv4Addr::new(10, 1, 255, 255));
        assert_eq!(network.prefix(), 16);
        Ok(())
    }

    #[test]
    fn contains_and_overlaps() -> Result<(), NetworkError> {
        let network: Ipv4Network = "10.0.0.0/24".parse()?;
        let overlap: Ipv4Network = "10.0.0.128/25".parse()?;
        let separate: Ipv4Network = "10.0.1.0/24".parse()?;
        assert!(network.contains(Ipv4Addr::new(10, 0, 0, 70)));
        assert!(network.overlaps(overlap));
        assert!(!network.overlaps(separate));
        Ok(())
    }

    #[test]
    fn iterates_all_addresses() -> Result<(), NetworkError> {
        let network: Ipv4Network = "192.0.2.0/30".parse()?;
        assert_eq!(network.iter().count(), 4);
        Ok(())
    }

    #[test]
    fn handles_boundary_prefixes() -> Result<(), NetworkError> {
        let all: Ipv4Network = "0.0.0.0/0".parse()?;
        let pair: Ipv4Network = "192.0.2.0/31".parse()?;
        let host: Ipv4Network = "192.0.2.1/32".parse()?;
        let last: Ipv4Network = "255.255.255.255/32".parse()?;
        assert!(all.contains(Ipv4Addr::BROADCAST));
        assert_eq!(pair.iter().count(), 2);
        assert_eq!(
            host.iter().collect::<Vec<_>>(),
            [Ipv4Addr::new(192, 0, 2, 1)]
        );
        assert_eq!(last.iter().collect::<Vec<_>>(), [Ipv4Addr::BROADCAST]);
        Ok(())
    }

    #[test]
    fn rejects_invalid_cidrs() {
        assert!("10.0.0.0/33".parse::<Ipv4Network>().is_err());
        assert!("10.0.0.0/24/1".parse::<Ipv4Network>().is_err());
        assert!("not-an-ip/24".parse::<Ipv4Network>().is_err());
        assert!("10.0.0.0/255.0.255.0".parse::<Ipv4Network>().is_err());
    }

    #[test]
    fn accepts_dotted_decimal_netmask() -> Result<(), NetworkError> {
        let network: Ipv4Network = "10.1.9.32/255.255.0.0".parse()?;
        assert_eq!(network.network(), Ipv4Addr::new(10, 1, 0, 0));
        assert_eq!(network.prefix(), 16);
        Ok(())
    }
}
