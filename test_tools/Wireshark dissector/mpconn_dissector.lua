local mpconnudp = Proto.new("mpconnudp", "MPCONN UDP")

-- local field_connid = ProtoField.uint64("mpconnudp.connid", "Conn ID", base.HEX)
local field_msg = ProtoField.uint8("mpconnudp.msg_type", "Msg type", base.HEX)
local field_seqnr = ProtoField.uint64("mpconnudp.seqnr", "Seq Nr.", base.DEC)
local field_peer_id = ProtoField.uint16("mpconnudp.peer_id", "Peer ID", base.DEC)
local field_msg_length = ProtoField.uint64("mpconnudp.msg_len", "Msg Len", base.DEC)
local field_ip = ProtoField.bytes("mpconnudp.ip", "IP")
local field_tunnel_ip = ProtoField.ipv4("mpconnudp.tun_ip", "Tunnel IP")
mpconnudp.fields = { field_msg, field_seqnr, field_peer_id, field_msg_length, field_ip, field_tunnel_ip }

-- Reverse of zigzag: https://docs.rs/bincode/latest/bincode/config/struct.VarintEncoding.html
function unzigzag(buffer)
    -- Decode u < 251
    if buffer(0, 1):le_uint() < 251 then
        return { 1, buffer(0, 1) }

        -- Decode 251 <= u < 2**16
    elseif buffer(0, 1):le_uint() == 251 then
        return { 3, buffer(1, 2) }

    -- Decode 2**16 <= u < 2**32
    elseif buffer(0, 1):le_uint() == 252 then
        return { 5, buffer(1, 4) }

    -- Decode 2**32 <= u < 2**64
        elseif buffer(0, 1):le_uint() == 253 then
        return { 9, buffer(1, 8) }
    end

end

-- the `dissector()` method is called by Wireshark when parsing our packets
-- `buffer` holds the UDP payload, all the bytes from our protocol
-- `tree` is the structure we see when inspecting/dissecting one particular packet
function mpconnudp.dissector(buffer, pinfo, tree)
    -- Changing the value in the protocol column (the Wireshark pane that displays a list of packets) 
    pinfo.cols.protocol = "MPCONN UDP"

    -- We label the entire UDP payload as being associated with our protocol
    local payload_tree = tree:add(mpconnudp, buffer())

    local message_type_pos = 0
    local message_type_len = 1
    local message_type_buffer = buffer(message_type_pos, message_type_len)
    local message_type = message_type_buffer:le_uint()
    payload_tree:add_le(field_msg, message_type_buffer)

    if message_type == 0 then
        local seqnr_pos = message_type_pos + message_type_len
        local seqnr_maxlen = 8
        local unzigzaged = unzigzag(buffer(seqnr_pos, seqnr_maxlen))
        local seqnr_len = unzigzaged[1]
        payload_tree:add_le(field_seqnr, unzigzaged[2])

        local peer_id_pos = seqnr_pos + seqnr_len
        local peer_id_maxlen = 3
        local peer_id_unzigzaged = unzigzag(buffer(peer_id_pos, peer_id_maxlen))
        local peer_id_len = peer_id_unzigzaged[1]
        payload_tree:add_le(field_peer_id, peer_id_unzigzaged[2])

        local msg_length_pos = peer_id_pos + peer_id_len
        local msg_length_maxlen = 8
        local msg_length_buffer = unzigzag(buffer(msg_length_pos, msg_length_maxlen))
        local msg_length_len = msg_length_buffer[1]
        payload_tree:add_le(field_msg_length, msg_length_buffer[2])

        local ip_pos = msg_length_pos + msg_length_len
        local ip_len = buffer:len() - ip_pos
        local ip_buffer = buffer(ip_pos, ip_len)

        --Dissector.get("eth_withoutfcs"):call(ip_buffer:tvb(), pinfo, tree)
        Dissector.get("ip"):call(ip_buffer:tvb(), pinfo, tree)
    else
        local peer_id_pos = message_type_pos + message_type_len
        local peer_id_maxlen = 3
        local peer_id_unzigzaged = unzigzag(buffer(peer_id_pos, peer_id_maxlen))
        local peer_id_len = peer_id_unzigzaged[1]
        payload_tree:add_le(field_peer_id, peer_id_unzigzaged[2])

        local tun_ip_pos = peer_id_pos + peer_id_len + 1 -- TODO: handle if the IP field is not filled
        local tun_ip_len = 4
        local tun_ip_buffer = buffer(tun_ip_pos, tun_ip_len)
        payload_tree:add(field_tunnel_ip, tun_ip_buffer)

    end
end

--we register our protocol on UDP port 10000
udp_table = DissectorTable.get("udp.port"):add(10000, mpconnudp)