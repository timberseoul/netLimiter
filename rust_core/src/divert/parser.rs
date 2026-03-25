use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Direction of the packet relative to the local machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

/// Parsed network packet information.
#[derive(Debug, Clone)]
pub struct ParsedPacket {
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8, // 6=TCP, 17=UDP
    pub length: usize,
    pub direction: Direction,
}

/// Parse raw IP packet bytes into structured info.
/// Returns None if the packet is not TCP/UDP or is malformed.
pub fn parse_packet(data: &[u8], outbound: bool) -> Option<ParsedPacket> {
    if data.is_empty() {
        return None;
    }

    let version = (data[0] >> 4) & 0x0F;

    match version {
        4 => parse_ipv4(data, outbound),
        6 => parse_ipv6(data, outbound),
        _ => None,
    }
}

fn parse_ipv4(data: &[u8], outbound: bool) -> Option<ParsedPacket> {
    if data.len() < 20 {
        return None;
    }

    let ihl = ((data[0] & 0x0F) as usize) * 4;
    if data.len() < ihl {
        return None;
    }

    let total_length = u16::from_be_bytes([data[2], data[3]]) as usize;
    let protocol = data[9];

    let src_ip = IpAddr::V4(Ipv4Addr::new(data[12], data[13], data[14], data[15]));
    let dst_ip = IpAddr::V4(Ipv4Addr::new(data[16], data[17], data[18], data[19]));

    // Only handle TCP (6) and UDP (17)
    if protocol != 6 && protocol != 17 {
        return None;
    }

    let transport = &data[ihl..];
    if transport.len() < 4 {
        return None;
    }

    let src_port = u16::from_be_bytes([transport[0], transport[1]]);
    let dst_port = u16::from_be_bytes([transport[2], transport[3]]);

    let direction = if outbound {
        Direction::Outbound
    } else {
        Direction::Inbound
    };

    Some(ParsedPacket {
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        protocol,
        length: total_length,
        direction,
    })
}

fn parse_ipv6(data: &[u8], outbound: bool) -> Option<ParsedPacket> {
    if data.len() < 40 {
        return None;
    }

    let payload_length = u16::from_be_bytes([data[4], data[5]]) as usize;
    let next_header = data[6]; // protocol

    // Only handle TCP (6) and UDP (17)
    if next_header != 6 && next_header != 17 {
        return None;
    }

    let src_bytes: [u8; 16] = data[8..24].try_into().ok()?;
    let dst_bytes: [u8; 16] = data[24..40].try_into().ok()?;

    let src_ip = IpAddr::V6(Ipv6Addr::from(src_bytes));
    let dst_ip = IpAddr::V6(Ipv6Addr::from(dst_bytes));

    let transport = &data[40..];
    if transport.len() < 4 {
        return None;
    }

    let src_port = u16::from_be_bytes([transport[0], transport[1]]);
    let dst_port = u16::from_be_bytes([transport[2], transport[3]]);

    let direction = if outbound {
        Direction::Outbound
    } else {
        Direction::Inbound
    };

    Some(ParsedPacket {
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        protocol: next_header,
        length: 40 + payload_length,
        direction,
    })
}
