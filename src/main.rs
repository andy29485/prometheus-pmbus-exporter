use clap::{crate_authors, crate_name, crate_version, Arg};
use i2cdev::core::*;
use i2cdev::linux::{LinuxI2CDevice, LinuxI2CError};
use prometheus_exporter::{self, prometheus::register_gauge_vec};
use std::env;
use std::net::IpAddr;

type PMBusResult<T> = Result<T, LinuxI2CError>;

// https://gist.github.com/otya128/1784473224a80f3ae453e8667b5fe8e5

const MOD2_ADDR: u16 = 0x59;
const MOD1_ADDR: u16 = 0x58;
const ATX_ADDR: u16 = 0x25;
const FRAME_ADDR: u16 = 0x56;
const PREFIX: &str = "fsp_twins_exporter";

const PAGE_CMD: u8 = 0x00;
const IVOLT_CMD: u8 = 0x88;
const OVOLT_EXP_CMD: u8 = 0x20;
const OVOLT_MANT_CMD: u8 = 0x8B;
const TEMP1_CMD: u8 = 0x8D;
const TEMP2_CMD: u8 = 0x8E;
const ICUR_CMD: u8 = 0x89;
const OCUR_CMD: u8 = 0x8C;
const IPOW_CMD: u8 = 0x97;
const OPOW_CMD: u8 = 0x96;
const FAN_SPEED_CMD: u8 = 0x90;

pub fn read_byte(dev: &str, addr: u16, com: u8) -> PMBusResult<u8> {
    let mut dev = LinuxI2CDevice::new(dev, addr)?;
    dev.set_smbus_pec(true)?;

    return dev.smbus_read_byte_data(com);
}

pub fn read_word(dev: &str, addr: u16, com: u8) -> PMBusResult<u16> {
    let mut dev = LinuxI2CDevice::new(dev, addr)?;
    dev.set_smbus_pec(true)?;

    return dev.smbus_read_word_data(com);
}

pub fn read_linear11(dev: &str, addr: u16, com: u8) -> PMBusResult<f32> {
    let mut dev = LinuxI2CDevice::new(dev, addr)?;
    dev.set_smbus_pec(true)?;

    let bits = dev.smbus_read_word_data(com)?;
    let exp = twos_comp((bits & 0xF800) >> 11, 5);  // high 5 bits
    let mant = twos_comp(bits & 0x7FF, 11);         // low 11 bits

    return Ok(mant as f32 * 2_f32.powi(exp as i32));
}

pub fn read_linear16(dev: &str, addr: u16, mant_com: u8, exp_com: u8) -> PMBusResult<f32> {
    let mut dev = LinuxI2CDevice::new(dev, addr)?;
    dev.set_smbus_pec(true)?;

    let mant = dev.smbus_read_word_data(mant_com)?;
    let exp = twos_comp(dev.smbus_read_byte_data(exp_com)? as u16, 5);

    return Ok((mant as f32) * 2_f32.powi(exp as i32));
}

pub fn twos_comp(val: u16, bits: usize) -> i16 {
    if val & (1<<(bits-1)) != 0 {
        ((val as i32) - (1_i32<<bits)) as i16
    } else {
        val as i16
    }
}

fn main() -> PMBusResult<()> {
    let matches = clap::Command::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!("\n"))
        .arg(
            Arg::new("addr")
                .short('l')
                .long("address")
                .env("PROMETHEUS_PMBUS_EXPORTER_ADDRESS")
                .help("exporter address")
                .default_value("0.0.0.0")
                .takes_value(true),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .env("PROMETHEUS_PMBUS_EXPORTER_PORT")
                .help("exporter port")
                .default_value("9986")
                .takes_value(true),
        )
        .arg(
            Arg::new("device")
                .env("PROMETHEUS_PMBUS_EXPORTER_DEVICE")
                .help("ic2 device to listen on")
                .takes_value(true),
        )
        .get_matches();

    let dev = matches.value_of("device").expect("device is required");

    let port = matches.value_of("port").unwrap();
    let port = port.parse::<u16>().expect("port must be a valid number");
    let addr = matches.value_of("addr").unwrap().parse::<IpAddr>().unwrap();
    let bind = (addr, port).into();

    let exporter = prometheus_exporter::start(bind).unwrap();

    let rpm_gague   = register_gauge_vec!(format!("{PREFIX}_fan_rpm"),		"Speed of the fan",				&["bus", "module"]).unwrap();
    let ivolt_gague = register_gauge_vec!(format!("{PREFIX}_input_voltage"),	"Input voltage from outlet",			&["bus", "module"]).unwrap();
    let icur_gague  = register_gauge_vec!(format!("{PREFIX}_input_current"),	"Input current (amp) from outlet",		&["bus", "module"]).unwrap();
    let ipow_gague  = register_gauge_vec!(format!("{PREFIX}_input_power"),		"Power (W) being drawn from outlet",		&["bus", "module"]).unwrap();
    let ovolt_gague = register_gauge_vec!(format!("{PREFIX}_output_voltage"),	"Voltage provided to PSU",			&["bus", "module"]).unwrap();
    let ocur_gague  = register_gauge_vec!(format!("{PREFIX}_output_current"),	"Current (amp) provided to the main PSU",	&["bus", "module"]).unwrap();
    let opow_gague  = register_gauge_vec!(format!("{PREFIX}_output_power"),	"Power (W) being drawn by the PSU",		&["bus", "module"]).unwrap();
    let temp_gague  = register_gauge_vec!(format!("{PREFIX}_temperature"),		"Temperature",					&["bus", "module", "sensor"]).unwrap();

    loop {
        // Will block until a new request comes in.
        let _guard = exporter.wait_request();

        for module in &["1", "2"] {
            let mod_addr = match module {
                &"1" => MOD1_ADDR,
                &"2" => MOD2_ADDR,
                _ => continue,
            };
            match rpm_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gague) => gague.set(read_word(&dev, mod_addr, FAN_SPEED_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match ivolt_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear16(&dev, MOD1_ADDR, IVOLT_CMD, OVOLT_EXP_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match icur_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear11(&dev, MOD1_ADDR, ICUR_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match ipow_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear11(&dev, MOD1_ADDR, IPOW_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match ovolt_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear16(&dev, MOD1_ADDR, OVOLT_MANT_CMD, OVOLT_EXP_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match ocur_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear11(&dev, MOD1_ADDR, OCUR_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            match opow_gague.get_metric_with_label_values(&[dev, &module]) {
                Ok(gauge) => gauge.set(read_linear11(&dev, MOD1_ADDR, OPOW_CMD)? as f64),
                Err(_) => todo!("This shouldn't happen, but add a log here"),
            }

            for temp_sensor in &["1", "2"] {
                let temp_cmd = match temp_sensor {
                    &"1" => TEMP1_CMD,
                    &"2" => TEMP2_CMD,
                    _ => continue,
                };
                match temp_gague.get_metric_with_label_values(&[dev, &module, &temp_sensor]) {
                    Ok(gauge) => gauge.set(read_linear11(&dev, MOD1_ADDR, temp_cmd)? as f64),
                    Err(_) => todo!("This shouldn't happen, but add a log here"),
                }
            }
        }
    }
}
