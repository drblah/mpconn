# mpconn
mpconn aims to be a flexible multi connectivity tunneling program, which supports both layer 2 and 3 tunneling over several transport protocols.

## What is multi connectivity?
Multi connectivity is the act of using two or more network paths in order to improve reliability and/or latency. Multi connectivity is especially useful when operating over an inherently unreliable physical layer such as wireless networks.
The simplest form of multi connectivity is packet duplication, where each network packet is duplicated and a copy is transmitted over each available physical link. By doing this, it is possible to take advantage of the fact that negative network conditions are often uncorrelated across the various network layers.


## Current implemented features
* Layer 2 tunneling (Ethernet over IP)
* Layer 3 tunneling (IP over IP)
* UDP Remote transport
* Multi connectivity via packet duplication on multiple network interfaces

## Usage
Each endpoint needs a host configuration. This configuration involves one or more remote endpoints to tunnel between, which Remote transport protocol to use and which layer to tunnel. Examples can be found in the test_tools directory along with bash scrpts for setting up network namespace based testing environments.
On each endpoint, run `mpconn --config <host-config>.json`

## Todo
* Hole punching for NAT traversal
* QUIC Remote transport
* Compression of payloads
* Packing
* Combine compression and packing
* Gateway management protocol
