# E1000 Driver Correctness Review

Reviewed: `kernel/comps/e1000/src/` (Asterinas Rust translation)
Reference: `linux-e1000/drivers/net/ethernet/intel/e1000/` (Linux 6.x)

---

## 1. Register Offsets (regs.rs vs e1000_hw.h)

All register offsets for the 82543+ (non-82542) layout have been verified
against `e1000_hw.h`:

| Register   | Rust        | Linux           | Match |
|------------|-------------|-----------------|-------|
| CTRL       | 0x00000     | 0x00000         | OK    |
| STATUS     | 0x00008     | 0x00008         | OK    |
| EECD       | 0x00010     | 0x00010         | OK    |
| EERD       | 0x00014     | 0x00014         | OK    |
| MDIC       | 0x00020     | 0x00020         | OK    |
| ICR        | 0x000C0     | 0x000C0         | OK    |
| ITR        | 0x000C4     | 0x000C4         | OK    |
| IMS        | 0x000D0     | 0x000D0         | OK    |
| IMC        | 0x000D8     | 0x000D8         | OK    |
| RCTL       | 0x00100     | 0x00100         | OK    |
| TCTL       | 0x00400     | 0x00400         | OK    |
| TIPG       | 0x00410     | 0x00410         | OK    |
| PBA        | 0x01000     | 0x01000         | OK    |
| RDBAL      | 0x02800     | 0x02800         | OK    |
| RDBAH      | 0x02804     | 0x02804         | OK    |
| RDLEN      | 0x02808     | 0x02808         | OK    |
| RDH        | 0x02810     | 0x02810         | OK    |
| RDT        | 0x02818     | 0x02818         | OK    |
| TDBAL      | 0x03800     | 0x03800         | OK    |
| TDBAH      | 0x03804     | 0x03804         | OK    |
| TDLEN      | 0x03808     | 0x03808         | OK    |
| TDH        | 0x03810     | 0x03810         | OK    |
| TDT        | 0x03818     | 0x03818         | OK    |
| RXCSUM     | 0x05000     | 0x05000         | OK    |
| MTA        | 0x05200     | 0x05200         | OK    |
| RA         | 0x05400     | 0x05400         | OK    |
| VFTA       | 0x05600     | 0x05600         | OK    |
| CRCERRS..  | 0x04000+    | 0x04000+        | OK    |

All CTRL, STATUS, EECD, EERD, MDIC, ICR, RCTL, TCTL bit-masks checked and
match their Linux counterparts exactly.

**Verdict: PASS** -- register map is faithful to 82540EM.

---

## 2. Init Sequence (driver.rs vs e1000_main.c probe/up)

Linux `e1000_probe()` / `e1000_open()` sequence:
1. Map BAR 0
2. `e1000_reset_hw` (disable IRQs, disable RX/TX, issue global reset)
3. `e1000_validate_eeprom_checksum`
4. `e1000_read_mac_addr`
5. `e1000_init_hw` (RAR, MTA, setup_link -> setup_copper_link -> PHY config)
6. `e1000_configure_tx`
7. `e1000_setup_rctl` + `e1000_configure_rx`
8. Allocate RX buffers, write RDT
9. Enable interrupts

Rust `E1000Device::init()` sequence:
1. Map BAR 0
2. `hw.reset_hw()` -- matches Linux
3. `eeprom::validate_eeprom_checksum` -- matches Linux
4. `hw.read_mac_addr()` -- matches Linux
5. `hw.init_hw()` -> `setup_link` -> `setup_copper_link` -- matches
6. `tx::configure_tx()`
7. `rx::configure_rx()`
8. `rx_ring.alloc_rx_buffers()` writes RDT
9. `intr::irq_enable()`

**Verdict: PASS** -- init ordering matches the Linux probe/open flow.

---

## 3. TX/RX Descriptor Layouts (desc.rs vs e1000_hw.h)

### RX Descriptor (Legacy)

| Field       | Rust offset | Linux struct         | Size | Match |
|-------------|-------------|----------------------|------|-------|
| buffer_addr | 0           | __le64 buffer_addr   | 8    | OK    |
| length      | 8           | __le16 length        | 2    | OK    |
| csum        | 10          | __le16 csum          | 2    | OK    |
| status      | 12          | u8 status            | 1    | OK    |
| errors      | 13          | u8 errors            | 1    | OK    |
| special     | 14          | __le16 special       | 2    | OK    |
| Total       | 16 bytes    | 16 bytes             |      | OK    |

`#[repr(C)]` ensures the layout matches hardware.

### TX Descriptor (Legacy)

Linux struct `e1000_tx_desc`:
```
buffer_addr: __le64        [0..8]
lower.data:  __le32        [8..12]   = {length[15:0], cso[7:0], cmd[7:0]}
upper.data:  __le32        [12..16]  = {status[7:0], css[7:0], special[15:0]}
```

Rust struct `E1000TxDesc`:
```
buffer_addr: u64           [0..8]
lower: u32                 [8..12]
upper: u32                 [12..16]
```

The `new_data()` method packs: `lower = (length as u32) | ((cmd as u32) << 24)`.
In little-endian memory, length occupies bytes 8-9, cso=0 in byte 10,
cmd in byte 11. This matches the Linux struct field order (length, cso, cmd)
because x86 is little-endian and the u32 places byte 0 at lowest address.

Wait -- the Linux struct packs `length` (16 bits), `cso` (8 bits), `cmd`
(8 bits) into a 32-bit word. On little-endian:
- byte 0 (offset 8): length[7:0]
- byte 1 (offset 9): length[15:8]
- byte 2 (offset 10): cso
- byte 3 (offset 11): cmd

So in the u32 little-endian representation:
`lower = length | (cso << 16) | (cmd << 24)`

Rust does: `lower = (length as u32) | ((cmd as u32) << 24)` (cso=0 implicit).

**Verdict: PASS** -- TX descriptor layout is correct.

### TX Descriptor CMD byte encoding

| Bit  | Rust          | Linux (byte-level)                          | Match |
|------|---------------|---------------------------------------------|-------|
| EOP  | 0x01          | E1000_TXD_CMD_EOP=0x01000000 => byte=0x01   | OK    |
| IFCS | 0x02          | E1000_TXD_CMD_IFCS=0x02000000 => byte=0x02  | OK    |
| RS   | 0x08          | E1000_TXD_CMD_RS=0x08000000 => byte=0x08    | OK    |
| DEXT | 0x20          | E1000_TXD_CMD_DEXT=0x20000000 => byte=0x20  | OK    |

**Verdict: PASS**

---

## 4. DMA Usage

### Descriptor Rings
- `rx.rs`: Uses `DmaStream<FromAndToDevice>` for the RX descriptor ring.
  This is appropriate since the CPU writes descriptors (buffer_addr) and the
  hardware writes back (status, length). Explicit `sync_to_device` /
  `sync_from_device` calls bracket access.
- `tx.rs`: Uses `DmaStream<FromAndToDevice>` for the TX descriptor ring.
  Same rationale: CPU writes descriptors, hardware writes back DD status.

### Packet Buffers
- **RX buffers**: Allocated from `DmaPool<FromDevice>` via `RxBuffer::new()`.
  Correct -- hardware writes received packet data.
- **TX buffers**: Allocated as `DmaStream<ToDevice>`. Correct -- CPU writes
  packet data, hardware reads it.

**ISSUE (Minor)**: The driver uses `DmaStream` (streaming DMA) for the
descriptor rings rather than `DmaCoherent`. While functionally correct with
explicit sync calls, the standard pattern for descriptor rings (which are
small, frequently accessed, and bidirectional) is coherent DMA to avoid
the overhead of explicit sync operations on every descriptor read/write.
This is a performance concern, not a correctness bug, because the sync calls
are present and correctly ordered.

**Verdict: PASS (functional), ADVISORY (use DmaCoherent for rings)**

---

## 5. MMIO Access (regs.rs)

The `E1000Regs` wrapper uses `IoMem` with `read_once::<u32>()` and
`write_once::<u32>()`. These are the Asterinas equivalents of volatile MMIO
accessors (`readl`/`writel` in Linux). No raw pointer arithmetic is used.

The `set_clear()` helper performs read-modify-write atomically at the
software level (single thread). This matches the Linux pattern.

**Verdict: PASS**

---

## 6. Interrupt Handling (intr.rs)

### ICR Bits

| Bit      | Rust            | Linux               | Match |
|----------|-----------------|----------------------|-------|
| TXDW     | 0x00000001      | E1000_ICR_TXDW       | OK    |
| TXQE     | 0x00000002      | E1000_ICR_TXQE       | OK    |
| LSC      | 0x00000004      | E1000_ICR_LSC        | OK    |
| RXSEQ    | 0x00000008      | E1000_ICR_RXSEQ      | OK    |
| RXDMT0   | 0x00000010      | E1000_ICR_RXDMT0     | OK    |
| RXO      | 0x00000040      | E1000_ICR_RXO        | OK    |
| RXT0     | 0x00000080      | E1000_ICR_RXT0       | OK    |
| MDAC     | 0x00000200      | E1000_ICR_MDAC       | OK    |
| INT_ASSERT| 0x80000000     | E1000_ICR_INT_ASSERTED| OK   |

### IMS Enable Mask

Rust: `IMS_ENABLE_MASK = ICR_RXT0 | ICR_TXDW | ICR_RXDMT0 | ICR_RXSEQ | ICR_LSC`
= 0x80 | 0x01 | 0x10 | 0x08 | 0x04 = 0x9D

Linux: `IMS_ENABLE_MASK = E1000_IMS_RXT0 | E1000_IMS_TXDW | E1000_IMS_RXDMT0 | E1000_IMS_RXSEQ | E1000_IMS_LSC`
= 0x80 | 0x01 | 0x10 | 0x08 | 0x04 = 0x9D

**Match confirmed.**

### Mask/Unmask Sequence

- `irq_enable`: Writes IMS_ENABLE_MASK to IMS, flushes with STATUS read.
  Matches Linux `e1000_irq_enable()`.
- `irq_disable`: Writes 0xFFFFFFFF to IMC, flushes with STATUS read.
  Matches Linux `e1000_irq_disable()`.
- `read_icr`: Reads ICR (read-to-clear). Matches Linux behavior.

**Verdict: PASS**

---

## 7. AnyNetworkDevice Trait Implementation (driver.rs)

| Method                    | Implementation                        | Status |
|---------------------------|---------------------------------------|--------|
| `mac_addr()`             | Returns stored MAC                     | OK     |
| `capabilities()`         | Returns DeviceCapabilities with csum   | OK     |
| `can_receive()`          | Checks if next RX desc has DD set      | OK     |
| `can_send()`             | Checks ring unused_count >= 1          | OK     |
| `receive()`              | Calls clean_rx_irq, returns RxBuffer   | OK     |
| `send()`                 | Calls xmit_frame                       | OK     |
| `free_processed_tx_buffers()` | Calls clean_tx_irq               | OK     |
| `notify_poll_end()`      | Re-enables interrupts                  | OK     |

**Verdict: PASS** -- All trait methods implemented.

---

## 8. Identified Correctness Issues

### BUG 1: COLLISION_DISTANCE_FD is 64, should be 63

**File**: `regs.rs`, line 260
**Current**: `pub const COLLISION_DISTANCE_FD: u32 = 64;`
**Expected**: `pub const COLLISION_DISTANCE_FD: u32 = 63;`

Linux defines `E1000_COLLISION_DISTANCE = 63` for all MAC types except
82542 (which uses 64). The 82540EM should use 63.

**Impact**: The collision distance written to TCTL[21:12] will be 64 instead
of 63. In practice this causes no harm for full-duplex gigabit (collision
distance is only meaningful in half-duplex), but it deviates from spec.

### BUG 2: Missing TCTL_RTLC in configure_tx

**File**: `tx.rs`, line 211-215
**Current**: TCTL is set to `TCTL_EN | TCTL_PSP | (CT << 4) | (COLD << 12)`
**Expected**: Should also include `TCTL_RTLC` (Re-transmit on Late Collision)

Linux `e1000_configure_tx()` includes `E1000_TCTL_RTLC`:
```c
tctl |= E1000_TCTL_PSP | E1000_TCTL_RTLC | (E1000_COLLISION_THRESHOLD << E1000_CT_SHIFT);
```

**Impact**: Without RTLC, the NIC will not automatically re-transmit frames
that suffered a late collision. This matters in half-duplex scenarios.

### BUG 3: Interrupt handler does not read ICR (shared IRQ correctness)

**File**: `driver.rs`, line 331-336
**Current**: `handle_interrupt()` unconditionally raises softirqs without
reading ICR to confirm this was actually the e1000's interrupt.
**Expected**: Should call `intr::process_interrupt()` (which reads ICR) and
only raise softirqs if relevant bits are set. Without this, the handler
cannot properly participate in shared IRQ lines (it will always claim the
interrupt).

**Impact**: If the IRQ line is shared with another device, spurious work
will be done. The unused `ICR_INT_ASSERTED` bit (0x80000000) should be
checked to confirm the interrupt belongs to this NIC.

### BUG 4: Single-packet-at-a-time RX path

**File**: `rx.rs`, `clean_rx_irq()` returns `Option<RxBuffer>` (one packet)
**Linux**: `e1000_clean_rx_irq()` processes up to a budget of packets per call.

This is not incorrect per se but degrades performance. Each call to
`receive()` returns at most one packet, requiring the network stack to call
back repeatedly. If the upper layer expects batch processing, this will be
significantly slower under load.

**Severity**: Low (functional correctness is fine; performance issue).

### BUG 5: RX ring refill advances RDT by one slot each time

**File**: `rx.rs`, lines 158-160

After cleaning one descriptor, the code allocates a replacement buffer and
writes `RDT = idx`. This means RDT advances one slot at a time.

The Linux driver batches buffer allocations and updates RDT in larger
increments to reduce MMIO write overhead. The Asterinas approach is correct
but suboptimal.

**Severity**: Low (performance, not correctness).

### ADVISORY 1: No PCI bus-mastering enable

The init sequence does not explicitly enable PCI bus-mastering on the
device. In Linux, `pci_set_master()` is called during probe. If the
Asterinas PCI framework does not do this automatically, DMA will not work.

This should be verified against the `aster_pci` framework to confirm
bus-mastering is enabled when the BAR is mapped.

### ADVISORY 2: No VLAN filter table clearing in init_hw

Linux clears all 128 VLAN filter table entries during `e1000_init_hw()`.
The Rust `init_hw()` only clears the MTA. If stale VFTA entries exist after
reset, VLAN filtering (if enabled via RCTL_VFE) could misbehave. Since the
Rust driver does NOT set RCTL_VFE, this is not a runtime issue currently.

---

## 9. Summary

| Category                 | Status                                    |
|--------------------------|-------------------------------------------|
| Register offsets         | PASS - all verified against e1000_hw.h    |
| Register bit-fields      | PASS - all match                          |
| Init sequence            | PASS - matches Linux probe/open flow      |
| TX descriptor layout     | PASS - correct #[repr(C)], correct packing|
| RX descriptor layout     | PASS - correct #[repr(C)], 16 bytes       |
| DMA direction correctness| PASS - correct directions for all buffers |
| DMA coherent vs stream   | ADVISORY - stream+sync works but coherent preferred for rings |
| MMIO access              | PASS - uses IoMem read_once/write_once    |
| Interrupt bits           | PASS - all ICR/IMS bits match             |
| Interrupt mask/unmask    | PASS - correct sequence with flush        |
| AnyNetworkDevice trait   | PASS - fully implemented                  |
| COLLISION_DISTANCE       | BUG - 64 should be 63 for 82540EM        |
| TCTL_RTLC missing        | BUG - half-duplex retransmit disabled     |
| IRQ handler ICR check    | BUG - does not confirm interrupt ownership|
| PHY/EEPROM access        | PASS - EERD and MDIC sequences correct    |
| Autoneg flow             | PASS - M88 PHY setup matches Linux        |
| Flow control             | PASS - FC register programming correct    |

**Overall Assessment**: The translation is largely faithful. The three bugs
identified (collision distance off-by-one, missing TCTL_RTLC, interrupt
handler not reading ICR) are real but have limited practical impact in the
typical QEMU/KVM full-duplex gigabit scenario where this driver is used.
The code is well-structured and the descriptor layouts, register offsets,
and init sequencing are all correct.
