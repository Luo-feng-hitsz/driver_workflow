export const meta = {
  name: 'linux-driver-to-asterinas',
  description: 'Translate a Linux C driver into an Asterinas Rust driver and integrate it into the kernel',
  whenToUse: 'When the user wants to port a Linux kernel driver to Asterinas OS (Rust). Pass the Linux driver path as args.',
  phases: [
    { title: 'Discover', detail: 'Analyze the Linux driver source to determine driver name, key files, PCI IDs, and architecture' },
    { title: 'Translate', detail: 'Per-module agents translate C driver to Rust in parallel, writes into asterinas/kernel/comps/' },
    { title: 'Assemble', detail: 'Write Cargo.toml and lib.rs that tie all translated modules together' },
    { title: 'Compile', detail: 'Run cargo check and fix any errors' },
    { title: 'Integrate', detail: 'Modify kernel Cargo.toml, workspace, and net iface init to register the new driver' },
    { title: 'Review', detail: 'Independent correctness + safety review' },
    { title: 'Test', detail: 'Boot kernel with new NIC, run wget bing.com, fix errors iteratively' },
  ],
}

const driverTarget = args
if (!driverTarget) {
  return { error: 'Pass the Linux driver source path as args (e.g. "./linux-r8169/drivers/net/ethernet/realtek")' }
}

const ASTERINAS = '/root/asterinas'
const DOCS_INDEX = `
Asterinas documentation (fetch ONLY when you hit a specific API question):
- DMA / MMIO / interrupts / I/O ports: https://asterinas.github.io/book/ostd/soundness/safe-kernel-peripheral-interactions.html
- Sync primitives (spinlock, preemption): https://asterinas.github.io/book/ostd/soundness/safe-kernel-logic.html
- OSTD overview: https://asterinas.github.io/book/ostd/index.html
`

const ASTERINAS_REFS = `
## Asterinas reference files (read these to understand the patterns you MUST follow)
- ${ASTERINAS}/kernel/comps/virtio/src/transport/pci/driver.rs — PciDriver trait impl
- ${ASTERINAS}/kernel/comps/virtio/src/device/network/ — virtio-net as reference template
- ${ASTERINAS}/kernel/comps/network/src/lib.rs — AnyNetworkDevice trait
- ${ASTERINAS}/kernel/comps/network/src/driver.rs — NetworkDevice registration
- ${ASTERINAS}/kernel/comps/pci/src/bus.rs — PCI bus, PciDriver trait
- ${ASTERINAS}/kernel/comps/pci/src/common_device.rs — PciCommonDevice API (BAR, IRQ)
- ${ASTERINAS}/kernel/comps/network/src/dma_pool.rs — DMA pool for packet buffers
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
          depends_on: { type: 'array', items: { type: 'string' }, description: 'Names of other modules this one imports from' },
        },
        required: ['name', 'purpose', 'source_files', 'depends_on'],
      },
      description: 'Planned Rust modules in translation order. Modules with no dependencies can be translated in parallel.',
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
Read ALL source files in that directory (use the Read tool on each file). Determine:
1. The driver name (short lowercase, e.g. "r8169", "e1000e")
2. A suitable Rust crate name (e.g. "aster-r8169")
3. A device registration label (e.g. "r8169-net")
4. The PCI vendor ID and supported device IDs — ONLY for the target chip (see below)
5. The primary/simplest chip variant to target first
6. Which source files map to which roles (registers, hardware ops, main driver, PHY, firmware)
7. A plan for Rust module decomposition — what modules to create, in what order, mapping from which C files.
   For each module, list which other modules it depends on (imports from).
   Modules with NO dependencies on each other can be translated in parallel.
   Keep each module focused — ONE module per concern. Typical modules:
   regs (register constants), desc (descriptor structs), hw (hardware init/reset/MMIO),
   phy (PHY config), tx (transmit path), rx (receive path), driver (PCI probe),
   device (AnyNetworkDevice impl).

## CRITICAL: target chip only

Many drivers support multiple chip variants with separate source files (e.g. e1000e has
82571.c, ich8lan.c, 80003es2lan.c). You MUST pick ONE target chip — the one QEMU emulates
by default — and ONLY plan modules that are needed for that chip.

- Do NOT create modules for other chip variants. Ignore their source files entirely.
- The hw module should only translate the init/reset/link functions relevant to the target chip.
- When a source file contains a mix of chip-specific and generic code, only translate the
  generic parts plus the target chip's code paths.
- For e1000e specifically, QEMU emulates an 82574L — only include ich8lan.c/ich8lan.h paths
  relevant to that, plus the generic mac.c/phy.c/netdev.c code. Skip 82571.c and 80003es2lan.c.
- Keep the total number of modules small (6-10 max). Fewer large modules are better than many
  small ones when each module translates a big C file.

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
log(`Modules planned: ${info.rust_modules.map(m => m.name).join(', ')}`)

// ─── Phase 1: Translate (per-module, dependency-aware) ───────────

phase('Translate')

const keyFilesDesc = Object.entries(info.key_files)
  .filter(([_, files]) => files && files.length > 0)
  .map(([role, files]) => `- ${role}: ${files.join(', ')}`)
  .join('\n')

const commonContext = `
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

${ASTERINAS_REFS}
${DOCS_INDEX}

## Output directory
${DRIVER_CRATE}/src/

## Key constraints
- ${info.target_chip} ONLY — ignore all other chip variants completely
- When a C source file contains code for multiple chip variants (switch/case on chip type,
  #ifdef blocks, separate functions per variant), ONLY translate the code paths for ${info.target_chip}.
  Stub or skip everything else.
- If the source file is entirely for a non-target chip variant, write a minimal placeholder module
  or skip it — do NOT translate thousands of lines of irrelevant code.
- unsafe minimization — use OSTD safe abstractions (IoMem, DMA, IRQ)
- Follow virtio-net patterns exactly for registration and device wrapping
- Use #[repr(C)] for hardware descriptor structs
- Do NOT fetch docs unless you hit a specific API question
`

// Build dependency graph: group modules into layers
const modules = info.rust_modules
const moduleNames = new Set(modules.map(m => m.name))
const completed = new Set()
const moduleResults = {}

// Translate in waves: each wave contains modules whose dependencies are all completed
let wave = 0
while (completed.size < modules.length) {
  wave++
  const ready = modules.filter(m =>
    !completed.has(m.name) &&
    m.depends_on.every(dep => completed.has(dep) || !moduleNames.has(dep))
  )

  if (ready.length === 0) {
    log(`Wave ${wave}: deadlock — remaining modules have circular dependencies, forcing sequential`)
    const remaining = modules.filter(m => !completed.has(m.name))
    for (const m of remaining) {
      const depModules = m.depends_on.filter(d => completed.has(d))
      const depInfo = depModules.length > 0
        ? `\n\nYou can reference these sibling modules that are already written:\n${depModules.map(d => `- ${DRIVER_CRATE}/src/${d}.rs`).join('\n')}`
        : ''

      await agent(
        `You are translating ONE module of a Linux network driver from C to Rust for Asterinas OS.

## Your module: ${m.name}.rs
Purpose: ${m.purpose}
Translate from these C source files: ${m.source_files.join(', ')}
${depInfo}

${commonContext}

Write ONLY this file: ${DRIVER_CRATE}/src/${m.name}.rs
Make sure to create the directory first: mkdir -p ${DRIVER_CRATE}/src/`,
        { label: `translate:${m.name}`, phase: 'Translate' }
      )
      completed.add(m.name)
      log(`Translated ${m.name}.rs (forced sequential)`)
    }
    break
  }

  log(`Wave ${wave}: translating ${ready.map(m => m.name).join(', ')} in parallel`)

  await parallel(ready.map(m => () => {
    const depModules = m.depends_on.filter(d => completed.has(d))
    const depInfo = depModules.length > 0
      ? `\n\nYou can reference these sibling modules that are already written — read them to ensure type/API consistency:\n${depModules.map(d => `- ${DRIVER_CRATE}/src/${d}.rs`).join('\n')}`
      : ''

    return agent(
      `You are translating ONE module of a Linux network driver from C to Rust for Asterinas OS.

## Your module: ${m.name}.rs
Purpose: ${m.purpose}
Translate from these C source files: ${m.source_files.join(', ')}
${depInfo}

${commonContext}

Write ONLY this file: ${DRIVER_CRATE}/src/${m.name}.rs
Make sure to create the directory first: mkdir -p ${DRIVER_CRATE}/src/`,
      { label: `translate:${m.name}`, phase: 'Translate' }
    )
  }))

  for (const m of ready) {
    completed.add(m.name)
  }
  log(`Wave ${wave} done: ${ready.map(m => m.name).join(', ')}`)
}

log('All modules translated')

// ─── Phase 1.5: Assemble Cargo.toml + lib.rs ─────────────────────

phase('Assemble')

const modDeclarations = modules.map(m => m.name).join(', ')

await agent(
  `You are assembling the final crate structure for the ${DRIVER_NAME} Asterinas driver.

The following module files have already been written by other agents at ${DRIVER_CRATE}/src/:
${modules.map(m => `- ${m.name}.rs — ${m.purpose}`).join('\n')}

## Task 1: Write Cargo.toml
Write ${DRIVER_CRATE}/Cargo.toml
- Read ${ASTERINAS}/kernel/comps/virtio/Cargo.toml as reference for workspace dependencies
- name = "${CRATE_NAME}"
- Add all dependencies that the module files use (read each .rs file to check imports)

## Task 2: Write lib.rs
Write ${DRIVER_CRATE}/src/lib.rs
- Read ALL the module .rs files first to understand their public APIs
- Add mod declarations for: ${modDeclarations}
- Add #[init_component] fn that registers the PCI driver
- Follow the pattern in ${ASTERINAS}/kernel/comps/virtio/src/lib.rs

${ASTERINAS_REFS}

## Key constraints
- Read every existing .rs file in ${DRIVER_CRATE}/src/ before writing lib.rs
- Ensure all use/import paths are consistent with what the modules actually export
- The init function must match what the driver.rs module exposes`,
  { label: 'assemble' }
)
log('Cargo.toml and lib.rs assembled')

// ─── Phase 1.75: Compile check + fix ────────────────────────────

phase('Compile')

const MAX_FIX_ATTEMPTS = 3
for (let attempt = 1; attempt <= MAX_FIX_ATTEMPTS; attempt++) {
  const checkResult = await agent(
    `Run cargo check on the ${DRIVER_NAME} driver crate and report the result.

## Steps
1. Run: cd ${ASTERINAS} && cargo check -p ${CRATE_NAME} 2>&1
2. If it compiles successfully (no errors), return exactly: {"success": true, "errors": ""}
3. If there are errors, return: {"success": false, "errors": "<the full error output>"}

Only report — do NOT fix anything.`,
    { label: `check:attempt-${attempt}`, phase: 'Compile', schema: {
      type: 'object',
      properties: {
        success: { type: 'boolean' },
        errors: { type: 'string' },
      },
      required: ['success', 'errors'],
    }}
  )

  if (!checkResult || checkResult.success) {
    log(`cargo check passed (attempt ${attempt})`)
    break
  }

  if (attempt === MAX_FIX_ATTEMPTS) {
    log(`cargo check still failing after ${MAX_FIX_ATTEMPTS} fix attempts — continuing anyway`)
    break
  }

  log(`cargo check failed (attempt ${attempt}), fixing...`)

  await agent(
    `The ${DRIVER_NAME} driver crate at ${DRIVER_CRATE}/ failed cargo check. Fix the errors.

## Errors
${checkResult.errors}

## Instructions
- Read the relevant .rs files in ${DRIVER_CRATE}/src/
- Fix ONLY the compilation errors — do not refactor or change logic
- Common issues: missing imports, type mismatches between modules, wrong API usage
- After fixing, the code should compile with: cd ${ASTERINAS} && cargo check -p ${CRATE_NAME}

${ASTERINAS_REFS}`,
    { label: `fix:attempt-${attempt}`, phase: 'Compile' }
  )
}

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
- It should call aster_network::get_device("${DEVICE_LABEL}")
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
- Cross-module consistency: types used across modules match

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

// ─── Phase 4: Test — boot kernel, wget bing.com, fix if broken ──

phase('Test')

const DNS_SERVER = '10.0.2.3'  // QEMU user-mode networking default DNS
const MAX_TEST_ATTEMPTS = 3

for (let attempt = 1; attempt <= MAX_TEST_ATTEMPTS; attempt++) {
  const testResult = await agent(
    `You are testing the ${DRIVER_NAME} network driver in Asterinas OS by booting QEMU and running wget.

## Steps

1. Kill any existing QEMU instances:
   pkill -f qemu-system 2>/dev/null; sleep 2

2. Boot the kernel with the ${DRIVER_NAME} NIC:
   cd ${ASTERINAS} && NIC=${DRIVER_NAME} LOG_LEVEL=info make run_kernel VNC_PORT=28 2>&1
   Wait for it to start (check that qemu-system process is running).
   Then wait ~20 seconds for boot.

3. Check serial log for driver probe success:
   grep -i "${DRIVER_NAME}\\|found.*NIC\\|MAC" ${ASTERINAS}/qemu-serial.log
   If the driver did NOT probe (no log lines), return {"success": false, "stage": "probe", "error": "driver did not probe"}

4. Install vncdotool if not available:
   pip3 install --break-system-packages vncdotool 2>/dev/null

5. Find the actual VNC port:
   ss -tlnp | grep qemu | grep -o ':\\([0-9]*\\)' to find the listening port

6. Send commands via VNC to configure DNS and run wget:
   vncdo -s localhost::<PORT> type "echo 'nameserver ${DNS_SERVER}' > /etc/resolv.conf"
   vncdo -s localhost::<PORT> key enter
   sleep 1
   vncdo -s localhost::<PORT> type "wget --timeout=15 -q -O /dev/null http://bing.com && echo WGET_SUCCESS || echo WGET_FAILED"
   vncdo -s localhost::<PORT> key enter

7. Wait 20 seconds, then check the output:
   Check ${ASTERINAS}/qemu.log for WGET_SUCCESS or WGET_FAILED or "stalled" or errors.

8. Kill QEMU after test:
   pkill -f "qemu-system.*vnc.*:28" 2>/dev/null

## Important notes
- The VNC display shows UEFI boot screen, but the shell is on virtconsole which also outputs to qemu.log via the mux chardev
- Check qemu.log (NOT qemu-serial.log) for wget output since that's where the mux stdio goes
- QEMU user-mode networking DNS is at 10.0.2.3

## Return
Return a JSON object:
- If wget succeeded: {"success": true, "output": "<relevant log lines>"}
- If wget failed/stalled: {"success": false, "stage": "wget", "error": "<relevant error or log lines showing what went wrong>"}
- If driver didn't probe: {"success": false, "stage": "probe", "error": "<details>"}
- If build failed: {"success": false, "stage": "build", "error": "<compiler errors>"}`,
    { label: `test:attempt-${attempt}`, phase: 'Test', schema: {
      type: 'object',
      properties: {
        success: { type: 'boolean' },
        stage: { type: 'string' },
        output: { type: 'string' },
        error: { type: 'string' },
      },
      required: ['success'],
    }}
  )

  if (!testResult || testResult.success) {
    log(`wget test passed (attempt ${attempt})`)
    break
  }

  if (attempt === MAX_TEST_ATTEMPTS) {
    log(`wget test still failing after ${MAX_TEST_ATTEMPTS} attempts — giving up`)
    break
  }

  log(`wget test failed at stage "${testResult.stage}" (attempt ${attempt}): ${testResult.error}`)
  log('Spawning fix agent...')

  await agent(
    `The ${DRIVER_NAME} network driver in Asterinas failed a wget test. Fix the issue.

## What happened
- Test stage: ${testResult.stage}
- Error: ${testResult.error}

## Common issues and fixes
- "driver did not probe": Check PCI device ID matching, init_component registration, Components.toml entry
- "stalled" during wget: Interrupt handler not registered, or not reading ICR to clear interrupt.
  The e1000 uses level-triggered INTx — you MUST read the ICR register in the interrupt handler to deassert the line.
  Check: is handle_interrupt() registered via IrqLine::alloc() + on_active() + IRQ_CHIP.map_gsi_pin_to()?
  Check: does handle_interrupt() read ICR before calling raise_receive_softirq()?
- "bad address": DNS not configured (this is handled by the test script, not a driver bug)
- "connection refused" or timeout: TX path might be broken, check tx descriptor ring and TCTL enable

## Instructions
- Read the driver source at ${DRIVER_CRATE}/src/
- Read the Asterinas PCI/IRQ framework:
  - ${ASTERINAS}/ostd/src/irq/top_half.rs (IrqLine API)
  - ${ASTERINAS}/ostd/src/arch/x86/irq/chip/mod.rs (IRQ_CHIP, map_gsi_pin_to)
  - ${ASTERINAS}/kernel/comps/pci/src/cfg_space.rs (PciCommonCfgOffset::InterruptLine = 0x3C)
- Fix ONLY the issue — do not refactor or change unrelated code
- After fixing, verify with: cd ${ASTERINAS} && NIC=${DRIVER_NAME} make run_kernel VNC_PORT=28 2>&1 | grep "^error" (should be empty)

${ASTERINAS_REFS}`,
    { label: `fix:attempt-${attempt}`, phase: 'Test' }
  )
}

return {
  driver_name: DRIVER_NAME,
  crate_name: CRATE_NAME,
  driver_crate: DRIVER_CRATE,
  target_chip: info.target_chip,
  modules: modules.map(m => m.name),
  message: `${DRIVER_NAME} driver translated, integrated, and tested. Check REVIEW-*.md for findings.`,
}
