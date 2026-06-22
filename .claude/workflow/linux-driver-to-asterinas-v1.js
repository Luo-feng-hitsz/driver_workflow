export const meta = {
  name: 'linux-driver-to-asterinas',
  description: 'Translate a Linux C driver into an Asterinas Rust driver and integrate it into the kernel',
  whenToUse: 'When the user wants to port a Linux kernel driver to Asterinas OS (Rust). Pass the Linux driver path as args.',
  phases: [
    { title: 'Discover', detail: 'Analyze the Linux driver source to determine driver name, key files, PCI IDs, and architecture' },
    { title: 'Translate', detail: 'Single agent sequentially translates C driver to Rust, writes into asterinas/kernel/comps/' },
    { title: 'Integrate', detail: 'Modify kernel Cargo.toml, workspace, and net iface init to register the new driver' },
    { title: 'Review', detail: 'Independent correctness + safety review' },
  ],
}

const driverTarget = args
if (!driverTarget) {
  return { error: 'Pass the Linux driver source path as args (e.g. "./linux-r8169/drivers/net/ethernet/realtek")' }
}

const ASTERINAS = './asterinas'
const DOCS_INDEX = `
Asterinas documentation (fetch ONLY when you hit a specific API question):
- DMA / MMIO / interrupts / I/O ports: https://asterinas.github.io/book/ostd/soundness/safe-kernel-peripheral-interactions.html
- Sync primitives (spinlock, preemption): https://asterinas.github.io/book/ostd/soundness/safe-kernel-logic.html
- OSTD overview: https://asterinas.github.io/book/ostd/index.html
`

const DISCOVER_SCHEMA = {
  type: 'object',
  properties: {
    driver_name: { type: 'string', description: 'Short lowercase name for the driver, e.g. "r8169", "e1000"' },
    crate_name: { type: 'string', description: 'Rust crate name, e.g. "aster-r8169", "aster-e1000"' },
    device_label: { type: 'string', description: 'Device label for registration, e.g. "r8169-net"' },
    pci_vendor: { type: 'string', description: 'PCI vendor ID hex string, e.g. "0x10EC"' },
    pci_devices: { type: 'array', items: { type: 'string' }, description: 'List of PCI device ID hex strings to support' },
    target_chip: { type: 'string', description: 'Primary chip model to target, e.g. "RTL8169", "82540EM"' },
    key_files: {
      type: 'object',
      properties: {
        registers: { type: 'array', items: { type: 'string' }, description: 'Header files defining register constants' },
        hw_ops: { type: 'array', items: { type: 'string' }, description: 'Files with hardware init/reset/MMIO ops' },
        main: { type: 'array', items: { type: 'string' }, description: 'Main driver file(s) with probe, tx/rx, descriptor setup' },
        phy: { type: 'array', items: { type: 'string' }, description: 'PHY configuration files (if any)' },
        firmware: { type: 'array', items: { type: 'string' }, description: 'Firmware loading files (if any)' },
      },
      required: ['registers', 'hw_ops', 'main'],
    },
    rust_modules: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          name: { type: 'string', description: 'Rust module file name without .rs, e.g. "regs", "hw", "desc"' },
          purpose: { type: 'string', description: 'What this module contains' },
          source_files: { type: 'array', items: { type: 'string' }, description: 'Which C source files it translates from' },
        },
        required: ['name', 'purpose', 'source_files'],
      },
      description: 'Planned Rust modules in translation order',
    },
    notes: { type: 'string', description: 'Any special considerations for this driver' },
  },
  required: ['driver_name', 'crate_name', 'device_label', 'pci_vendor', 'pci_devices', 'target_chip', 'key_files', 'rust_modules'],
}

// ─── Phase 0: Discover ──────────────────────────────────────────

phase('Discover')

const info = await agent(
  `You are analyzing a Linux network driver to plan its translation to Rust for Asterinas OS.

## Source
The Linux driver source code is at: ${driverTarget}/

## Your task
Read ALL source files in that directory. Determine:
1. The driver name (short lowercase, e.g. "r8169", "e1000")
2. A suitable Rust crate name (e.g. "aster-r8169")
3. A device registration label (e.g. "r8169-net")
4. The PCI vendor ID and supported device IDs
5. The primary/simplest chip variant to target first
6. Which source files map to which roles (registers, hardware ops, main driver, PHY, firmware)
7. A plan for Rust module decomposition — what modules to create, in what order, mapping from which C files

Pick the simplest widely-used chip variant as the initial target.
Only include files that actually exist in the source directory.

Return structured output matching the schema.`,
  { label: 'discover', schema: DISCOVER_SCHEMA }
)

if (!info || !info.driver_name) {
  return { error: 'Discovery phase failed to analyze the driver source' }
}

const DRIVER_NAME = info.driver_name
const CRATE_NAME = info.crate_name
const DEVICE_LABEL = info.device_label
const DRIVER_CRATE = `${ASTERINAS}/kernel/comps/${DRIVER_NAME}`

log(`Discovered driver: ${DRIVER_NAME} (${info.target_chip}), PCI ${info.pci_vendor}, crate: ${CRATE_NAME}`)

// ─── Phase 1: Translate ──────────────────────────────────────────

phase('Translate')

const moduleList = info.rust_modules.map(
  (m, i) => `### Step ${i + 2}: ${m.name}.rs — ${m.purpose}
Translate from: ${m.source_files.join(', ')}
Write: \${DRIVER_CRATE}/src/${m.name}.rs`
).join('\n\n')

const keyFilesDesc = Object.entries(info.key_files)
  .filter(([_, files]) => files && files.length > 0)
  .map(([role, files]) => `- ${role}: ${files.join(', ')}`)
  .join('\n')

await agent(
  `You are translating a Linux network driver from C to Rust for Asterinas OS.

## Driver info
- Name: ${DRIVER_NAME}
- Target chip: ${info.target_chip}
- PCI vendor: ${info.pci_vendor}, devices: ${info.pci_devices.join(', ')}
- Device label: ${DEVICE_LABEL}
${info.notes ? '- Notes: ' + info.notes : ''}

## Source
Linux driver C code is at: ${driverTarget}/
Key files by role:
${keyFilesDesc}

## Reference
Read these Asterinas source files to understand the patterns you MUST follow:
- ${ASTERINAS}/kernel/comps/virtio/src/transport/pci/driver.rs — PciDriver trait impl
- ${ASTERINAS}/kernel/comps/virtio/src/device/network/ — virtio-net as reference template
- ${ASTERINAS}/kernel/comps/network/src/lib.rs — AnyNetworkDevice trait
- ${ASTERINAS}/kernel/comps/network/src/driver.rs — NetworkDevice registration
- ${ASTERINAS}/kernel/comps/pci/src/bus.rs — PCI bus, PciDriver trait
- ${ASTERINAS}/kernel/comps/pci/src/common_device.rs — PciCommonDevice API (BAR, IRQ)
- ${ASTERINAS}/kernel/comps/network/src/dma_pool.rs — DMA pool for packet buffers

${DOCS_INDEX}

## Output
Write ALL files into: ${DRIVER_CRATE}/
mkdir -p ${DRIVER_CRATE}/src/

## Translation steps (do them IN ORDER, each file builds on the previous):

### Step 1: Cargo.toml
Write ${DRIVER_CRATE}/Cargo.toml
Use virtio's Cargo.toml as reference for workspace dependencies.
name = "${CRATE_NAME}"

${moduleList}

### Final step: lib.rs — Component entry
#[init_component] fn, register PCI driver, mod declarations for all modules above.
Write ${DRIVER_CRATE}/src/lib.rs

## Key constraints
- ${info.target_chip} ONLY — ignore all other chip variants
- unsafe minimization — use OSTD safe abstractions (IoMem, DMA, IRQ)
- Follow virtio-net patterns exactly for registration and device wrapping
- Use #[repr(C)] for hardware descriptor structs
- Do NOT fetch docs unless you hit a specific API question`,
  { label: 'translate' }
)
log('Translation done')

// ─── Phase 2: Integrate into kernel ──────────────────────────────

phase('Integrate')

await agent(
  `Integrate the new ${DRIVER_NAME} driver crate into the Asterinas kernel build and network stack.

## What was just created
A new driver crate at: ${DRIVER_CRATE}/
Crate name: ${CRATE_NAME}
Device label: ${DEVICE_LABEL}

## Files to modify

### 1. Root Cargo.toml: ${ASTERINAS}/Cargo.toml
Add "kernel/comps/${DRIVER_NAME}" to workspace members list (near where "kernel/comps/virtio" appears).
Add ${CRATE_NAME} = { path = "kernel/comps/${DRIVER_NAME}" } to [workspace.dependencies].

### 2. Kernel Cargo.toml: ${ASTERINAS}/kernel/Cargo.toml
Add ${CRATE_NAME}.workspace = true (near where aster-virtio appears).

### 3. Network iface init: ${ASTERINAS}/kernel/src/net/iface/init.rs
This file currently hardcodes virtio-net. Modify it to also support ${DRIVER_NAME}:
- Add a new_${DRIVER_NAME}() function similar to new_virtio()
- It should call aster_network::get_device("${DEVICE_LABEL}") (or whatever device name the ${DRIVER_NAME} driver registers with)
- Register recv/send callbacks for ${DRIVER_NAME}
- The ${DRIVER_NAME} iface should be created as an EtherIface just like virtio

### 4. Network iface mod.rs: ${ASTERINAS}/kernel/src/net/iface/mod.rs
Export the new ${DRIVER_NAME} iface function if needed.

### 5. IP common: ${ASTERINAS}/kernel/src/net/socket/ip/common.rs
This file references virtio_iface(). Make it work with ${DRIVER_NAME} too — e.g., check for ${DRIVER_NAME} iface as fallback if virtio is not present.

## Reference
Read the current content of each file before modifying.
Follow the existing patterns exactly — the goal is minimal, consistent changes.`,
  { label: 'integrate' }
)
log('Integration done')

// ─── Phase 3: Review ─────────────────────────────────────────────

phase('Review')

await parallel([
  () => agent(
    `Review the ${DRIVER_NAME} driver code at ${DRIVER_CRATE}/src/ for CORRECTNESS.

Read ALL .rs files in the driver crate.
Also read the Linux source at ${driverTarget}/ to verify the translation is faithful.

Check:
- Register offsets match the original C headers for ${info.target_chip}
- Init sequence matches the original probe() flow
- TX/RX descriptor layouts match hardware spec (#[repr(C)], correct byte offsets)
- DMA usage: DmaCoherent for rings, DmaStream for packet buffers
- MMIO: uses BarAccess read_once/write_once, not raw pointers
- Interrupt: correct ICR bits, proper mask/unmask sequence
- AnyNetworkDevice trait fully implemented

Write: ${DRIVER_CRATE}/REVIEW-correctness.md`,
    { label: 'review:correct', phase: 'Review' }
  ),
  () => agent(
    `Review the kernel integration changes for the ${DRIVER_NAME} driver.

Check these files for correctness:
- ${ASTERINAS}/Cargo.toml — workspace members and dependencies
- ${ASTERINAS}/kernel/Cargo.toml — kernel dependency
- ${ASTERINAS}/kernel/src/net/iface/init.rs — ${DRIVER_NAME} iface setup
- ${ASTERINAS}/kernel/src/net/iface/mod.rs — exports
- ${ASTERINAS}/kernel/src/net/socket/ip/common.rs — ${DRIVER_NAME} fallback

Compare with how virtio-net is integrated. Flag any inconsistencies.
Also check that the ${DRIVER_NAME} driver's lib.rs init_component and device registration are consistent with what init.rs expects.

Write: ${DRIVER_CRATE}/REVIEW-integration.md`,
    { label: 'review:integration', phase: 'Review' }
  ),
])
log('Review done')

return {
  driver_name: DRIVER_NAME,
  crate_name: CRATE_NAME,
  driver_crate: DRIVER_CRATE,
  target_chip: info.target_chip,
  message: `${DRIVER_NAME} driver translated and integrated. Check REVIEW-*.md for findings.`,
}
