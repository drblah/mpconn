#!/bin/bash

set -eE -o functrace

failure() {
  local lineno=$1
  local msg=$2
  echo "Failed at $lineno: $msg"
}
trap 'failure ${LINENO} "$BASH_COMMAND"' ERR

##
# Based on https://ops.tips/blog/using-network-namespaces-and-bridge-to-isolate-servers/
##
# Create namespaces
ip netns add host1
ip netns add host2

ip netns add client1
ip netns add client2

# Create veth pairs
ip link add veth1 type veth peer name br-veth1
ip link add veth2 type veth peer name br-veth2
ip link add veth3 type veth peer name br-veth3

ip link add gw_veth1 type veth peer name br-gw-veth1
ip link add gw_veth2 type veth peer name br-gw-veth2

# Client pairs
ip link add cli_veth1 type veth peer name br-cl-veth1
ip link add cli_veth2 type veth peer name br-cl-veth2

# Associate the veth pairs with the namespaces
ip link set veth1 netns host1
ip link set veth2 netns host1
ip link set gw_veth1 netns host1

ip link set veth3 netns host2
ip link set gw_veth2 netns host2

ip link set cli_veth1 netns client1
ip link set cli_veth2 netns client2

# Assign IPs
ip netns exec host1 \
  ip addr add 172.16.200.2/24 dev veth1

ip netns exec host1 \
  ip addr add 172.16.200.3/24 dev veth2

ip netns exec host1 \
  ip addr add 172.16.210.2/24 dev gw_veth1

ip netns exec host2 \
  ip addr add 172.16.200.4/24 dev veth3

ip netns exec host2 \
  ip addr add 172.16.210.3/24 dev gw_veth2

ip netns exec client1 \
  ip add add 172.16.210.10/24 dev cli_veth1

ip netns exec client2 \
  ip add add 172.16.210.11/24 dev cli_veth2

# Create bridge
ip link add name mptun_bridge type bridge
ip link set mptun_bridge up

ip link add name bridge_gw1 type bridge
ip link add name bridge_gw2 type bridge

ip link set bridge_gw1 up
ip link set bridge_gw2 up

# Bring up all interfaces
ip link set br-veth1 up
ip link set br-veth2 up
ip link set br-veth3 up

ip link set br-gw-veth1 up
ip link set br-gw-veth2 up

ip link set br-cl-veth1 up
ip link set br-cl-veth2 up

ip netns exec host1 \
  ip link set veth1 up

ip netns exec host1 \
  ip link set veth2 up

ip netns exec host1 \
  ip link set gw_veth1 up

ip netns exec host2 \
  ip link set veth3 up

ip netns exec host2 \
  ip link set gw_veth2 up

ip netns exec client1 \
  ip link set cli_veth1 up

ip netns exec client2 \
  ip link set cli_veth2 up

# Disable GRO to prevent wrong TCP and UDP checksums
ip netns exec client1 \
  ethtool --offload cli_veth1 rx off

ip netns exec client1 \
  ethtool --offload cli_veth1 tx off

ip netns exec client2 \
  ethtool --offload cli_veth2 rx off

ip netns exec client2 \
  ethtool --offload cli_veth2 tx off


# Add br-veth* to the bridge
ip link set br-veth1 master mptun_bridge
ip link set br-veth2 master mptun_bridge
ip link set br-veth3 master mptun_bridge

ip link set br-gw-veth1 master bridge_gw1
ip link set br-gw-veth2 master bridge_gw2

ip link set br-cl-veth1 master bridge_gw1
ip link set br-cl-veth2 master bridge_gw2

# Assign address to bridge
ip addr add 172.16.200.1/24 brd + dev mptun_bridge
ip addr add 172.16.210.1/24 brd + dev bridge_gw1
ip addr add 172.16.210.1/24 brd + dev bridge_gw2

# Start interactive consoles for each namespace

tmux \
	new-session  "ip netns exec host1 bash" \; \
	split-window "ip netns exec host2 bash" \; \
	select-layout even-vertical

P1=$!


wait $P1

## Clean up the bridge and namespaces
ip netns delete host1
ip netns delete host2
ip netns delete client1
ip netns delete client2
ip link delete dev mptun_bridge
ip link delete dev bridge_gw1
ip link delete dev bridge_gw2