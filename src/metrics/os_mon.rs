use crate::InfluxDBArgs;
use atlas_common::async_runtime as rt;

use chrono::{DateTime, Utc};
use influxdb::InfluxDbWriteable;
use std::time::Duration;
use sysinfo::{Networks, Process, System};

/// OS Metrics
pub const OS_CPU_USER: &str = "OS_CPU_USER";

pub const OS_RAM_USAGE: &str = "OS_RAM_USAGE";

pub const OS_NETWORK_UP: &str = "OS_NETWORK_UP";

pub const OS_NETWORK_DOWN: &str = "OS_NETWORK_DOWN";

#[derive(InfluxDbWriteable)]
pub struct MetricCPUReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    #[influxdb(tag)]
    cpu: i32,
    value: f64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricRAMUsage {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricNetworkSpeed {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: f64,
}

pub fn launch_os_mon(influx_args: InfluxDBArgs) {
    std::thread::spawn(move || {
        metric_thread_loop(influx_args);
    });
}

/// The metrics thread. Collects all values from the
pub fn metric_thread_loop(influx_args: InfluxDBArgs) {
    let InfluxDBArgs {
        ip,
        db_name,
        user,
        password,
        node_id,
        extra,
    } = influx_args;

    let mut client = influxdb::Client::new(ip.to_string(), db_name);

    client = client.with_auth(user, password);

    let host_name = format!("{node_id:?}");

    let extra = extra.unwrap_or(String::from("None"));

    let mut sys = System::new_all();

    let mut networks = Networks::new_with_refreshed_list();

    let pid = sysinfo::get_current_pid().expect("Failed to get PID");

    loop {
        let mut readings = Vec::new();

        let cpu_readings = read_cpu_usage(&mut sys, host_name.clone(), extra.clone());

        cpu_readings
            .into_iter()
            .for_each(|reading| readings.push(reading.into_query(OS_CPU_USER)));

        let (tx_speed, rx_speed) =
            read_network_speed(&mut networks, host_name.clone(), extra.clone());

        readings.push(tx_speed.into_query(OS_NETWORK_UP));
        readings.push(rx_speed.into_query(OS_NETWORK_DOWN));

        if let Some(process_info) = sys.process(pid) {
            let ram_usage = read_ram_usage(process_info, host_name.clone(), extra.clone());

            readings.push(ram_usage.into_query(OS_RAM_USAGE));
        }

        let _result =
            rt::block_on(client.query(readings)).expect("Failed to write metrics to influxdb");

        std::thread::sleep(Duration::from_millis(250));

        sys.refresh_all();
        networks.refresh(false);
    }
}

fn read_cpu_usage(sys: &mut System, host_name: String, extra: String) -> Vec<MetricCPUReading> {
    let mut readings = Vec::new();

    for (cpu_core, cpu) in sys.cpus().iter().enumerate() {
        let reading = MetricCPUReading {
            time: Utc::now(),
            host: host_name.clone(),
            extra: extra.clone(),
            cpu: cpu_core as i32,
            value: cpu.cpu_usage() as f64 / 100.0,
        };

        readings.push(reading);
    }

    readings
}

fn read_network_speed(
    networks: &mut Networks,
    host_name: String,
    extra: String,
) -> (MetricNetworkSpeed, MetricNetworkSpeed) {
    let mut tx_speed = 0;
    let mut rx_speed = 0;

    for (_inf, network) in networks.iter() {
        let received_bytes = network.received();
        let transmitted_bytes = network.transmitted();

        tx_speed += received_bytes as i64;
        rx_speed += transmitted_bytes as i64;
    }

    (
        MetricNetworkSpeed {
            time: Utc::now(),
            host: host_name.clone(),
            extra: extra.clone(),
            value: tx_speed as f64,
        },
        MetricNetworkSpeed {
            time: Utc::now(),
            host: host_name,
            extra,
            value: rx_speed as f64,
        },
    )
}

fn read_ram_usage(sys_ref: &Process, host_name: String, extra: String) -> MetricRAMUsage {
    let used_memory = sys_ref.memory();

    MetricRAMUsage {
        time: Utc::now(),
        host: host_name,
        extra,
        value: used_memory as i64,
    }
}
