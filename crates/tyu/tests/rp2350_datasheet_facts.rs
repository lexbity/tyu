//! Datasheet-facts pin for the RP2350 board pack (P8, docs-as-tests).
//!
//! Every device row in `platforms/rp2350/platform.toml` is checked against
//! offsets and names transcribed from RP-008373-DS-2
//! (devdocs/HW-datasheets/RP-008373-DS-2-rp2350-datasheet.pdf): address map
//! Tables 13/14, system IRQs Table 95, and the per-peripheral "List of
//! registers" tables. A descriptor row that drifts from the datasheet — or a
//! datasheet-verified row dropped from the descriptor — fails here, so the
//! G14c presence guard cannot pass on wrong tables again.
//!
//! Deliberate descriptor exclusions (rationale in
//! platforms/rp2350/docs/descriptor-notes.md) are exclusions here too:
//! IO_BANK0 IRQSUMMARY/PROC1/DORMANT_WAKE banks, UART/SPI identification
//! registers, QMI timing/translation rows, SIO INTERP*/TMDS* accelerators,
//! (PADS SWCLK/SWD debug pads are included: they are ordinary pad controls.)
//!
//! The semantic annotations (access/write_kind/read_kind/irq) are pinned
//! for the rows whose semantics the audit found dangerous to get wrong.

use tyu::platform::desc::{load_descriptor, Descriptor};

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rp2350_descriptor() -> Descriptor {
    let root = workspace_root();
    load_descriptor(&root, "rp2350")
        .expect("loading rp2350 descriptor")
        .expect("rp2350 pack must carry a descriptor v2")
}

/// (offset, name) pairs transcribed from the datasheet register lists.
type Rows = &'static [(u32, &'static str)];

const GPIO_ROWS: Rows = &[
    (0x0, "GPIO0_STATUS"), (0x4, "GPIO0_CTRL"), (0x8, "GPIO1_STATUS"), (0xc, "GPIO1_CTRL"),
    (0x10, "GPIO2_STATUS"), (0x14, "GPIO2_CTRL"), (0x18, "GPIO3_STATUS"), (0x1c, "GPIO3_CTRL"),
    (0x20, "GPIO4_STATUS"), (0x24, "GPIO4_CTRL"), (0x28, "GPIO5_STATUS"), (0x2c, "GPIO5_CTRL"),
    (0x30, "GPIO6_STATUS"), (0x34, "GPIO6_CTRL"), (0x38, "GPIO7_STATUS"), (0x3c, "GPIO7_CTRL"),
    (0x40, "GPIO8_STATUS"), (0x44, "GPIO8_CTRL"), (0x48, "GPIO9_STATUS"), (0x4c, "GPIO9_CTRL"),
    (0x50, "GPIO10_STATUS"), (0x54, "GPIO10_CTRL"), (0x58, "GPIO11_STATUS"),
    (0x5c, "GPIO11_CTRL"), (0x60, "GPIO12_STATUS"), (0x64, "GPIO12_CTRL"),
    (0x68, "GPIO13_STATUS"), (0x6c, "GPIO13_CTRL"), (0x70, "GPIO14_STATUS"),
    (0x74, "GPIO14_CTRL"), (0x78, "GPIO15_STATUS"), (0x7c, "GPIO15_CTRL"),
    (0x80, "GPIO16_STATUS"), (0x84, "GPIO16_CTRL"), (0x88, "GPIO17_STATUS"),
    (0x8c, "GPIO17_CTRL"), (0x90, "GPIO18_STATUS"), (0x94, "GPIO18_CTRL"),
    (0x98, "GPIO19_STATUS"), (0x9c, "GPIO19_CTRL"), (0xa0, "GPIO20_STATUS"),
    (0xa4, "GPIO20_CTRL"), (0xa8, "GPIO21_STATUS"), (0xac, "GPIO21_CTRL"),
    (0xb0, "GPIO22_STATUS"), (0xb4, "GPIO22_CTRL"), (0xb8, "GPIO23_STATUS"),
    (0xbc, "GPIO23_CTRL"), (0xc0, "GPIO24_STATUS"), (0xc4, "GPIO24_CTRL"),
    (0xc8, "GPIO25_STATUS"), (0xcc, "GPIO25_CTRL"), (0xd0, "GPIO26_STATUS"),
    (0xd4, "GPIO26_CTRL"), (0xd8, "GPIO27_STATUS"), (0xdc, "GPIO27_CTRL"),
    (0xe0, "GPIO28_STATUS"), (0xe4, "GPIO28_CTRL"), (0xe8, "GPIO29_STATUS"),
    (0xec, "GPIO29_CTRL"), (0xf0, "GPIO30_STATUS"), (0xf4, "GPIO30_CTRL"),
    (0xf8, "GPIO31_STATUS"), (0xfc, "GPIO31_CTRL"), (0x100, "GPIO32_STATUS"),
    (0x104, "GPIO32_CTRL"), (0x108, "GPIO33_STATUS"), (0x10c, "GPIO33_CTRL"),
    (0x110, "GPIO34_STATUS"), (0x114, "GPIO34_CTRL"), (0x118, "GPIO35_STATUS"),
    (0x11c, "GPIO35_CTRL"), (0x120, "GPIO36_STATUS"), (0x124, "GPIO36_CTRL"),
    (0x128, "GPIO37_STATUS"), (0x12c, "GPIO37_CTRL"), (0x130, "GPIO38_STATUS"),
    (0x134, "GPIO38_CTRL"), (0x138, "GPIO39_STATUS"), (0x13c, "GPIO39_CTRL"),
    (0x140, "GPIO40_STATUS"), (0x144, "GPIO40_CTRL"), (0x148, "GPIO41_STATUS"),
    (0x14c, "GPIO41_CTRL"), (0x150, "GPIO42_STATUS"), (0x154, "GPIO42_CTRL"),
    (0x158, "GPIO43_STATUS"), (0x15c, "GPIO43_CTRL"), (0x160, "GPIO44_STATUS"),
    (0x164, "GPIO44_CTRL"), (0x168, "GPIO45_STATUS"), (0x16c, "GPIO45_CTRL"),
    (0x170, "GPIO46_STATUS"), (0x174, "GPIO46_CTRL"), (0x178, "GPIO47_STATUS"),
    (0x17c, "GPIO47_CTRL"), (0x230, "INTR0"), (0x234, "INTR1"), (0x238, "INTR2"),
    (0x23c, "INTR3"), (0x240, "INTR4"), (0x244, "INTR5"), (0x248, "PROC0_INTE0"),
    (0x24c, "PROC0_INTE1"), (0x250, "PROC0_INTE2"), (0x254, "PROC0_INTE3"),
    (0x258, "PROC0_INTE4"), (0x25c, "PROC0_INTE5"), (0x260, "PROC0_INTF0"),
    (0x264, "PROC0_INTF1"), (0x268, "PROC0_INTF2"), (0x26c, "PROC0_INTF3"),
    (0x270, "PROC0_INTF4"), (0x274, "PROC0_INTF5"), (0x278, "PROC0_INTS0"),
    (0x27c, "PROC0_INTS1"), (0x280, "PROC0_INTS2"), (0x284, "PROC0_INTS3"),
    (0x288, "PROC0_INTS4"), (0x28c, "PROC0_INTS5"),
];

const PADS_ROWS: Rows = &[
    (0x0, "VOLTAGE_SELECT"), (0x4, "GPIO0"), (0x8, "GPIO1"), (0xc, "GPIO2"), (0x10, "GPIO3"),
    (0x14, "GPIO4"), (0x18, "GPIO5"), (0x1c, "GPIO6"), (0x20, "GPIO7"), (0x24, "GPIO8"),
    (0x28, "GPIO9"), (0x2c, "GPIO10"), (0x30, "GPIO11"), (0x34, "GPIO12"), (0x38, "GPIO13"),
    (0x3c, "GPIO14"), (0x40, "GPIO15"), (0x44, "GPIO16"), (0x48, "GPIO17"), (0x4c, "GPIO18"),
    (0x50, "GPIO19"), (0x54, "GPIO20"), (0x58, "GPIO21"), (0x5c, "GPIO22"), (0x60, "GPIO23"),
    (0x64, "GPIO24"), (0x68, "GPIO25"), (0x6c, "GPIO26"), (0x70, "GPIO27"), (0x74, "GPIO28"),
    (0x78, "GPIO29"), (0x7c, "GPIO30"), (0x80, "GPIO31"), (0x84, "GPIO32"), (0x88, "GPIO33"),
    (0x8c, "GPIO34"), (0x90, "GPIO35"), (0x94, "GPIO36"), (0x98, "GPIO37"), (0x9c, "GPIO38"),
    (0xa0, "GPIO39"), (0xa4, "GPIO40"), (0xa8, "GPIO41"), (0xac, "GPIO42"), (0xb0, "GPIO43"),
    (0xb4, "GPIO44"), (0xb8, "GPIO45"), (0xbc, "GPIO46"), (0xc0, "GPIO47"), (0xc4, "SWCLK"),
    (0xc8, "SWD"),
];
const UART_ROWS: Rows = &[
    (0x0, "UARTDR"), (0x4, "UARTRSR"), (0x18, "UARTFR"), (0x20, "UARTILPR"), (0x24, "UARTIBRD"),
    (0x28, "UARTFBRD"), (0x2c, "UARTLCR_H"), (0x30, "UARTCR"), (0x34, "UARTIFLS"),
    (0x38, "UARTIMSC"), (0x3c, "UARTRIS"), (0x40, "UARTMIS"), (0x44, "UARTICR"),
    (0x48, "UARTDMACR"),
];

const SPI_ROWS: Rows = &[
    (0x0, "SSPCR0"), (0x4, "SSPCR1"), (0x8, "SSPDR"), (0xc, "SSPSR"), (0x10, "SSPCPSR"),
    (0x14, "SSPIMSC"), (0x18, "SSPRIS"), (0x1c, "SSPMIS"), (0x20, "SSPICR"), (0x24, "SSPDMACR"),
];

const I2C_ROWS: Rows = &[
    (0x0, "IC_CON"), (0x4, "IC_TAR"), (0x8, "IC_SAR"), (0x10, "IC_DATA_CMD"),
    (0x14, "IC_SS_SCL_HCNT"), (0x18, "IC_SS_SCL_LCNT"), (0x1c, "IC_FS_SCL_HCNT"),
    (0x20, "IC_FS_SCL_LCNT"), (0x2c, "IC_INTR_STAT"), (0x30, "IC_INTR_MASK"),
    (0x34, "IC_RAW_INTR_STAT"), (0x38, "IC_RX_TL"), (0x3c, "IC_TX_TL"), (0x40, "IC_CLR_INTR"),
    (0x44, "IC_CLR_RX_UNDER"), (0x48, "IC_CLR_RX_OVER"), (0x4c, "IC_CLR_TX_OVER"),
    (0x50, "IC_CLR_RD_REQ"), (0x54, "IC_CLR_TX_ABRT"), (0x58, "IC_CLR_RX_DONE"),
    (0x5c, "IC_CLR_ACTIVITY"), (0x60, "IC_CLR_STOP_DET"), (0x64, "IC_CLR_START_DET"),
    (0x68, "IC_CLR_GEN_CALL"), (0x6c, "IC_ENABLE"), (0x70, "IC_STATUS"), (0x74, "IC_TXFLR"),
    (0x78, "IC_RXFLR"), (0x7c, "IC_SDA_HOLD"), (0x80, "IC_TX_ABRT_SOURCE"),
    (0x84, "IC_SLV_DATA_NACK_ONLY"), (0x88, "IC_DMA_CR"), (0x8c, "IC_DMA_TDLR"),
    (0x90, "IC_DMA_RDLR"), (0x94, "IC_SDA_SETUP"), (0x98, "IC_ACK_GENERAL_CALL"),
    (0x9c, "IC_ENABLE_STATUS"), (0xa0, "IC_FS_SPKLEN"), (0xa8, "IC_CLR_RESTART_DET"),
    (0xf4, "IC_COMP_PARAM_1"), (0xf8, "IC_COMP_VERSION"), (0xfc, "IC_COMP_TYPE"),
];

const TIMER_ROWS: Rows = &[
    (0x0, "TIMEHW"), (0x4, "TIMELW"), (0x8, "TIMEHR"), (0xc, "TIMELR"), (0x10, "ALARM0"),
    (0x14, "ALARM1"), (0x18, "ALARM2"), (0x1c, "ALARM3"), (0x20, "ARMED"), (0x24, "TIMERAWH"),
    (0x28, "TIMERAWL"), (0x2c, "DBGPAUSE"), (0x30, "PAUSE"), (0x34, "LOCKED"), (0x38, "SOURCE"),
    (0x3c, "INTR"), (0x40, "INTE"), (0x44, "INTF"), (0x48, "INTS"),
];

const QMI_ROWS: Rows = &[
    (0x0, "DIRECT_CSR"), (0x4, "DIRECT_TX"), (0x8, "DIRECT_RX"),
];

const PIO_ROWS: Rows = &[
    (0x0, "CTRL"), (0x4, "FSTAT"), (0x8, "FDEBUG"), (0xc, "FLEVEL"), (0x10, "TXF0"),
    (0x14, "TXF1"), (0x18, "TXF2"), (0x1c, "TXF3"), (0x20, "RXF0"), (0x24, "RXF1"),
    (0x28, "RXF2"), (0x2c, "RXF3"), (0x30, "IRQ"), (0x34, "IRQ_FORCE"),
    (0x38, "INPUT_SYNC_BYPASS"), (0x3c, "DBG_PADOUT"), (0x40, "DBG_PADOE"),
    (0x44, "DBG_CFGINFO"), (0x48, "INSTR_MEM0"), (0x4c, "INSTR_MEM1"), (0x50, "INSTR_MEM2"),
    (0x54, "INSTR_MEM3"), (0x58, "INSTR_MEM4"), (0x5c, "INSTR_MEM5"), (0x60, "INSTR_MEM6"),
    (0x64, "INSTR_MEM7"), (0x68, "INSTR_MEM8"), (0x6c, "INSTR_MEM9"), (0x70, "INSTR_MEM10"),
    (0x74, "INSTR_MEM11"), (0x78, "INSTR_MEM12"), (0x7c, "INSTR_MEM13"), (0x80, "INSTR_MEM14"),
    (0x84, "INSTR_MEM15"), (0x88, "INSTR_MEM16"), (0x8c, "INSTR_MEM17"), (0x90, "INSTR_MEM18"),
    (0x94, "INSTR_MEM19"), (0x98, "INSTR_MEM20"), (0x9c, "INSTR_MEM21"), (0xa0, "INSTR_MEM22"),
    (0xa4, "INSTR_MEM23"), (0xa8, "INSTR_MEM24"), (0xac, "INSTR_MEM25"), (0xb0, "INSTR_MEM26"),
    (0xb4, "INSTR_MEM27"), (0xb8, "INSTR_MEM28"), (0xbc, "INSTR_MEM29"), (0xc0, "INSTR_MEM30"),
    (0xc4, "INSTR_MEM31"), (0xc8, "SM0_CLKDIV"), (0xcc, "SM0_EXECCTRL"),
    (0xd0, "SM0_SHIFTCTRL"), (0xd4, "SM0_ADDR"), (0xd8, "SM0_INSTR"), (0xdc, "SM0_PINCTRL"),
    (0xe0, "SM1_CLKDIV"), (0xe4, "SM1_EXECCTRL"), (0xe8, "SM1_SHIFTCTRL"), (0xec, "SM1_ADDR"),
    (0xf0, "SM1_INSTR"), (0xf4, "SM1_PINCTRL"), (0xf8, "SM2_CLKDIV"), (0xfc, "SM2_EXECCTRL"),
    (0x100, "SM2_SHIFTCTRL"), (0x104, "SM2_ADDR"), (0x108, "SM2_INSTR"), (0x10c, "SM2_PINCTRL"),
    (0x110, "SM3_CLKDIV"), (0x114, "SM3_EXECCTRL"), (0x118, "SM3_SHIFTCTRL"),
    (0x11c, "SM3_ADDR"), (0x120, "SM3_INSTR"), (0x124, "SM3_PINCTRL"), (0x128, "RXF0_PUTGET0"),
    (0x12c, "RXF0_PUTGET1"), (0x130, "RXF0_PUTGET2"), (0x134, "RXF0_PUTGET3"),
    (0x138, "RXF1_PUTGET0"), (0x13c, "RXF1_PUTGET1"), (0x140, "RXF1_PUTGET2"),
    (0x144, "RXF1_PUTGET3"), (0x148, "RXF2_PUTGET0"), (0x14c, "RXF2_PUTGET1"),
    (0x150, "RXF2_PUTGET2"), (0x154, "RXF2_PUTGET3"), (0x158, "RXF3_PUTGET0"),
    (0x15c, "RXF3_PUTGET1"), (0x160, "RXF3_PUTGET2"), (0x164, "RXF3_PUTGET3"),
    (0x168, "GPIOBASE"), (0x16c, "INTR"), (0x170, "IRQ0_INTE"), (0x174, "IRQ0_INTF"),
    (0x178, "IRQ0_INTS"), (0x17c, "IRQ1_INTE"), (0x180, "IRQ1_INTF"), (0x184, "IRQ1_INTS"),
];

const SIO_ROWS: Rows = &[
    (0x0, "CPUID"), (0x4, "GPIO_IN"), (0x8, "GPIO_HI_IN"), (0x10, "GPIO_OUT"),
    (0x14, "GPIO_HI_OUT"), (0x18, "GPIO_OUT_SET"), (0x1c, "GPIO_HI_OUT_SET"),
    (0x20, "GPIO_OUT_CLR"), (0x24, "GPIO_HI_OUT_CLR"), (0x28, "GPIO_OUT_XOR"),
    (0x2c, "GPIO_HI_OUT_XOR"), (0x30, "GPIO_OE"), (0x34, "GPIO_HI_OE"), (0x38, "GPIO_OE_SET"),
    (0x3c, "GPIO_HI_OE_SET"), (0x40, "GPIO_OE_CLR"), (0x44, "GPIO_HI_OE_CLR"),
    (0x48, "GPIO_OE_XOR"), (0x4c, "GPIO_HI_OE_XOR"), (0x50, "FIFO_ST"), (0x54, "FIFO_WR"),
    (0x58, "FIFO_RD"), (0x5c, "SPINLOCK_ST"), (0x100, "SPINLOCK0"), (0x104, "SPINLOCK1"),
    (0x108, "SPINLOCK2"), (0x10c, "SPINLOCK3"), (0x110, "SPINLOCK4"), (0x114, "SPINLOCK5"),
    (0x118, "SPINLOCK6"), (0x11c, "SPINLOCK7"), (0x120, "SPINLOCK8"), (0x124, "SPINLOCK9"),
    (0x128, "SPINLOCK10"), (0x12c, "SPINLOCK11"), (0x130, "SPINLOCK12"), (0x134, "SPINLOCK13"),
    (0x138, "SPINLOCK14"), (0x13c, "SPINLOCK15"), (0x140, "SPINLOCK16"), (0x144, "SPINLOCK17"),
    (0x148, "SPINLOCK18"), (0x14c, "SPINLOCK19"), (0x150, "SPINLOCK20"), (0x154, "SPINLOCK21"),
    (0x158, "SPINLOCK22"), (0x15c, "SPINLOCK23"), (0x160, "SPINLOCK24"), (0x164, "SPINLOCK25"),
    (0x168, "SPINLOCK26"), (0x16c, "SPINLOCK27"), (0x170, "SPINLOCK28"), (0x174, "SPINLOCK29"),
    (0x178, "SPINLOCK30"), (0x17c, "SPINLOCK31"), (0x180, "DOORBELL_OUT_SET"),
    (0x184, "DOORBELL_OUT_CLR"), (0x188, "DOORBELL_IN_SET"), (0x18c, "DOORBELL_IN_CLR"),
    (0x190, "PERI_NONSEC"), (0x1a0, "RISCV_SOFTIRQ"), (0x1a4, "MTIME_CTRL"), (0x1b0, "MTIME"),
    (0x1b4, "MTIMEH"), (0x1b8, "MTIMECMP"), (0x1bc, "MTIMECMPH"),
];

const FACT_DEVICES: &[(&str, &str, u64, Rows)] = &[
    ("GPIO", "gpio0", 0x40028000, GPIO_ROWS),
    ("PADS", "pads_bank0", 0x40038000, PADS_ROWS),
    ("UART", "uart0", 0x40070000, UART_ROWS),
    ("UART", "uart1", 0x40078000, UART_ROWS),
    ("SPI", "spi0", 0x40080000, SPI_ROWS),
    ("SPI", "spi1", 0x40088000, SPI_ROWS),
    ("I2C", "i2c0", 0x40090000, I2C_ROWS),
    ("I2C", "i2c1", 0x40098000, I2C_ROWS),
    ("TIMER", "timer0", 0x400b0000, TIMER_ROWS),
    ("TIMER", "timer1", 0x400b8000, TIMER_ROWS),
    ("QMI", "qmi", 0x400d0000, QMI_ROWS),
    ("PIO", "pio0", 0x50200000, PIO_ROWS),
    ("PIO", "pio1", 0x50300000, PIO_ROWS),
    ("SIO", "sio", 0xd0000000, SIO_ROWS),
];

#[test]
fn every_device_row_matches_the_datasheet() {
    let desc = rp2350_descriptor();

    // The 14 modeled devices must all be present, at their datasheet bases,
    // inside apertures that exist.
    assert_eq!(desc.devices.len(), FACT_DEVICES.len());

    for (map, instance, base, rows) in FACT_DEVICES {
        let dev = desc
            .devices
            .iter()
            .find(|d| &d.map == map && &d.instance == instance)
            .unwrap_or_else(|| panic!("device {map}/{instance} missing from descriptor"));
        let aperture = desc
            .aperture(dev.aperture)
            .unwrap_or_else(|| panic!("device {map}/{instance}: unknown aperture"));
        let aperture_base = aperture.base.expect("bus aperture base");
        let absolute = aperture_base + dev.base_offset as u64;
        assert_eq!(
            absolute, *base,
            "device {map}/{instance} at {absolute:#x}, datasheet says {base:#x}"
        );

        // Every datasheet row must be present with the declared width, and
        // nothing else may be declared for the device.
        assert_eq!(
            dev.registers.len(),
            rows.len(),
            "device {map}/{instance}: row count drifted from the datasheet"
        );
        for (off, name) in rows.iter() {
            let reg = dev
                .registers
                .iter()
                .find(|r| r.offset == *off)
                .unwrap_or_else(|| panic!("device {map}/{instance}: missing row at {off:#x}"));
            assert_eq!(
                reg.name.as_str(),
                *name,
                "device {map}/{instance} row at {off:#x}: name drifted"
            );
            assert_eq!(reg.width, 32, "device {map}/{instance} row {name}: width");
            assert_eq!(
                reg.atomic_max, 32,
                "device {map}/{instance} row {name}: atomic_max"
            );
        }
    }
}

#[test]
fn dangerous_semantics_are_pinned() {
    let desc = rp2350_descriptor();
    let find = |instance: &str, off: u32| {
        desc.devices
            .iter()
            .find(|d| d.instance == instance)
            .unwrap()
            .registers
            .iter()
            .find(|r| r.offset == off)
            .unwrap_or_else(|| panic!("{instance} row at {off:#x}"))
    };

    use tyu::platform::desc::{ReadKind, WriteKind};

    // IO_BANK0: STATUS first, CTRL second (swapping them was the audit's
    // GPIO finding); INTR0-5 are write-1-to-clear raw status.
    assert_eq!(find("gpio0", 0x000).name.as_str(), "GPIO0_STATUS");
    assert_eq!(find("gpio0", 0x004).name.as_str(), "GPIO0_CTRL");
    assert!(matches!(find("gpio0", 0x230).write_kind, WriteKind::W1c));

    // TIMER0: the w1c register is INTR (the draft's TIMERALARM* rows do not
    // exist), ARMED is w1c, and the alarms sit at 0x10..0x1c.
    assert_eq!(find("timer0", 0x03c).name.as_str(), "INTR");
    assert!(matches!(find("timer0", 0x03c).write_kind, WriteKind::W1c));
    assert!(matches!(find("timer0", 0x020).write_kind, WriteKind::W1c));
    assert_eq!(find("timer0", 0x020).name.as_str(), "ARMED");

    // SIO: RP2350 offsets with the GPIO_HI bank interleaved, XOR aliases are
    // xor (not w1c — an RMW bic on the XOR alias computes the wrong value),
    // and the FIFO pop is effectful.
    let sio = desc.devices.iter().find(|d| d.instance == "sio").unwrap();
    let xor_rows: Vec<_> = sio
        .registers
        .iter()
        .filter(|r| r.name.as_str().contains("XOR"))
        .collect();
    assert_eq!(xor_rows.len(), 4);
    for r in &xor_rows {
        assert!(matches!(r.write_kind, WriteKind::Xor), "{}", r.name);
    }
    let fifo_rd = sio
        .registers
        .iter()
        .find(|r| r.name.as_str() == "FIFO_RD")
        .unwrap();
    assert!(matches!(fifo_rd.read_kind, ReadKind::Effectful));

    // UART0: DR pops the RX FIFO; ICR is w1c.
    assert!(matches!(find("uart0", 0x000).read_kind, ReadKind::Effectful));
    assert!(matches!(find("uart0", 0x044).write_kind, WriteKind::W1c));

    // PIO: the SM-IRQ flags register sits at 0x30 (0x24 is RXF1) and the
    // RX FIFOs pop on read.
    assert_eq!(find("pio0", 0x030).name.as_str(), "IRQ");
    assert!(matches!(find("pio0", 0x020).read_kind, ReadKind::Effectful));
}

/// System IRQ numbers (DS2 Table 95): the pack's first draft shipped with
/// RP2040 numbering, so every 1:1 irq claim is pinned.
#[test]
fn irq_numbers_match_table_95() {
    let desc = rp2350_descriptor();
    let irq_of = |instance: &str, off: u32| {
        desc.devices
            .iter()
            .find(|d| d.instance == instance)
            .unwrap()
            .registers
            .iter()
            .find(|r| r.offset == off)
            .unwrap()
            .irq
            .unwrap()
    };
    assert_eq!(irq_of("uart0", 0x03c), 33); // UART0_IRQ
    assert_eq!(irq_of("uart1", 0x03c), 34); // UART1_IRQ
    assert_eq!(irq_of("spi0", 0x018), 31); // SPI0_IRQ
    assert_eq!(irq_of("spi1", 0x018), 32); // SPI1_IRQ
    assert_eq!(irq_of("i2c0", 0x02c), 36); // I2C0_IRQ
    assert_eq!(irq_of("i2c1", 0x02c), 37); // I2C1_IRQ
    assert_eq!(irq_of("gpio0", 0x230), 21); // IO_IRQ_BANK0
    assert_eq!(irq_of("pio0", 0x170), 15); // PIO0_IRQ_0
    assert_eq!(irq_of("pio1", 0x170), 17); // PIO1_IRQ_0
    assert_eq!(irq_of("timer0", 0x010), 0); // TIMER0_IRQ_0
    assert_eq!(irq_of("timer1", 0x010), 4); // TIMER1_IRQ_0
}

#[test]
fn memory_map_and_apertures_match_the_datasheet() {
    let desc = rp2350_descriptor();

    // SRAM is 520 kB (DS2 Tables 11-12): 512 KiB striped banks 0-7 plus two
    // 4 KiB non-striped banks. The first board draft declared 0x84000, which
    // put the boot stack pointer past physical SRAM.
    let sram = desc.region("SRAM").expect("SRAM region");
    assert_eq!(sram.origin, 0x2000_0000);
    assert_eq!(sram.length, 0x0008_2000);

    let ap = |name: &str| {
        desc.apertures
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("aperture {name} missing"))
    };
    let apb = ap("apb");
    assert_eq!(apb.base, Some(0x4000_0000));
    assert_eq!(apb.size, 0x0100_0000);
    let pio = ap("pio");
    assert_eq!(pio.base, Some(0x5020_0000));
    assert_eq!(pio.size, 0x0020_0000);
    let sio = ap("sio");
    assert_eq!(sio.base, Some(0xd000_0000));

    // No aperture aliases a [memory] region unless declared scratch; the
    // board pack declares none (the QEMU scratch lives in the runtime
    // descriptors, D-14).
    for w in &desc.apertures {
        assert!(!w.scratch, "aperture {} must not be scratch", w.name);
    }
}

#[test]
fn metal_trust_enumerates_every_asm_backed_word() {
    let desc = rp2350_descriptor();
    for word in [
        "platform.gpio.init",
        "platform.gpio.write",
        "platform.gpio.read",
        "platform.uart.init",
        "platform.uart.tx",
        "platform.uart.rx",
        "platform.time.now_us",
        "platform.time.reboot",
        "testio.write-byte",
        "testio.write-str",
        "testio.exit",
        "platform.mem.region-create",
        "platform.mem.region-alloc",
        "platform.mem.region-reset",
        "platform.mem.region-destroy",
    ] {
        assert!(
            desc.metal_trust.iter().any(|w| w == word),
            "asm-backed word '{word}' missing from metal.trust"
        );
    }
}
