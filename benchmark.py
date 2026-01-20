#!/usr/bin/env python3
"""
Performance Comparison: Replication vs Erasure Coding

Generates detailed benchmark metrics for DynamicFS redundancy strategies.
"""

import time
from dataclasses import dataclass
from enum import Enum
from typing import Dict, List, Tuple


class RedundancyType(Enum):
    REPLICATION = "Replication (3x)"
    ERASURE_CODING = "Erasure Coding (4+2)"


@dataclass
class PerformanceMetrics:
    """Metrics for a single redundancy strategy"""
    redundancy_type: RedundancyType
    data_size_gb: int
    storage_overhead: float  # 3.0 for replication, 1.5 for EC
    write_latency_ms: float
    read_latency_ms: float
    encode_cpu_percent: float
    decode_cpu_percent: float
    network_io_gb: float
    rebuild_time_seconds: float
    monthly_cost_dollars: float
    failure_tolerance: int
    
    def __str__(self) -> str:
        return (
            f"{self.redundancy_type.value}:\n"
            f"  Storage Overhead: {self.storage_overhead}x\n"
            f"  Write Latency: {self.write_latency_ms:.1f}ms\n"
            f"  Read Latency: {self.read_latency_ms:.1f}ms\n"
            f"  Encode CPU: {self.encode_cpu_percent:.1f}%\n"
            f"  Decode CPU: {self.decode_cpu_percent:.1f}%\n"
            f"  Network I/O: {self.network_io_gb:.2f}GB\n"
            f"  Rebuild Time: {self.rebuild_time_seconds:.2f}s\n"
            f"  Monthly Cost: ${self.monthly_cost_dollars:,.0f}\n"
            f"  Failure Tolerance: {self.failure_tolerance} disk(s)"
        )


class PerformanceBenchmark:
    """Benchmark comparison utilities"""
    
    # Constants
    DISK_BANDWIDTH_MBPS = 100  # MB/s per disk
    NETWORK_BANDWIDTH_MBPS = 5000  # 40Gbps datacenter
    REED_SOLOMON_DECODE_RATE = 50  # MB/s decode rate
    DISK_FAILURE_RATE = 0.01  # 1% AFR
    STORAGE_COST_PER_TB_MONTH = 10  # $/TB/month
    
    @staticmethod
    def benchmark_replication(data_size_gb: int) -> PerformanceMetrics:
        """Benchmark replication (3x copies)"""
        data_size_mb = data_size_gb * 1024
        data_size_bytes = data_size_gb * 1024 * 1024 * 1024
        
        # Write: 3 parallel copies
        write_latency_ms = (data_size_mb / PerformanceBenchmark.DISK_BANDWIDTH_MBPS) * 1.5  # 1.5x for overhead
        
        # Read: pick one copy
        read_latency_ms = (data_size_mb / PerformanceBenchmark.DISK_BANDWIDTH_MBPS) * 1.1  # 1.1x for overhead
        
        # Encode CPU: just memcpy
        encode_cpu = 0.5
        decode_cpu = 0.5
        
        # Network I/O: 3x data
        network_io_gb = data_size_gb * 3
        
        # Rebuild: copy from one disk
        rebuild_time = (data_size_mb / PerformanceBenchmark.DISK_BANDWIDTH_MBPS) * 1.2
        
        # Monthly cost: 3x storage
        storage_gb = data_size_gb * 3
        monthly_cost = (storage_gb / 1024) * PerformanceBenchmark.STORAGE_COST_PER_TB_MONTH
        
        return PerformanceMetrics(
            redundancy_type=RedundancyType.REPLICATION,
            data_size_gb=data_size_gb,
            storage_overhead=3.0,
            write_latency_ms=write_latency_ms,
            read_latency_ms=read_latency_ms,
            encode_cpu_percent=encode_cpu,
            decode_cpu_percent=decode_cpu,
            network_io_gb=network_io_gb,
            rebuild_time_seconds=rebuild_time,
            monthly_cost_dollars=monthly_cost,
            failure_tolerance=2,
        )
    
    @staticmethod
    def benchmark_erasure_coding(data_size_gb: int) -> PerformanceMetrics:
        """Benchmark erasure coding (4+2)"""
        data_size_mb = data_size_gb * 1024
        
        # Write: split into 4 shards, encode 2 parity
        encode_time_ms = (data_size_mb / PerformanceBenchmark.REED_SOLOMON_DECODE_RATE)
        write_latency_ms = (data_size_mb / (PerformanceBenchmark.DISK_BANDWIDTH_MBPS / 1.5)) + encode_time_ms  # 6 disks
        
        # Read: read 4 shards, decode
        decode_time_ms = (data_size_mb / PerformanceBenchmark.REED_SOLOMON_DECODE_RATE)
        read_latency_ms = (data_size_mb / (PerformanceBenchmark.DISK_BANDWIDTH_MBPS * 2)) + decode_time_ms  # parallel reads
        
        # Encode CPU: GF operations
        encode_cpu = 15.0
        decode_cpu = 12.0
        
        # Network I/O: 1.5x data (4 data + 2 parity)
        network_io_gb = data_size_gb * 1.5
        
        # Rebuild: read 4 shards, compute, write 1 shard
        rebuild_time = (data_size_mb / PerformanceBenchmark.REED_SOLOMON_DECODE_RATE) * 2 + 50  # decode + write
        
        # Monthly cost: 1.5x storage
        storage_gb = data_size_gb * 1.5
        monthly_cost = (storage_gb / 1024) * PerformanceBenchmark.STORAGE_COST_PER_TB_MONTH
        
        return PerformanceMetrics(
            redundancy_type=RedundancyType.ERASURE_CODING,
            data_size_gb=data_size_gb,
            storage_overhead=1.5,
            write_latency_ms=write_latency_ms,
            read_latency_ms=read_latency_ms,
            encode_cpu_percent=encode_cpu,
            decode_cpu_percent=decode_cpu,
            network_io_gb=network_io_gb,
            rebuild_time_seconds=rebuild_time / 1000.0,  # convert to seconds
            monthly_cost_dollars=monthly_cost,
            failure_tolerance=2,
        )
    
    @staticmethod
    def compare_metrics(data_size_gb: int) -> str:
        """Generate comparison report"""
        repl = PerformanceBenchmark.benchmark_replication(data_size_gb)
        ec = PerformanceBenchmark.benchmark_erasure_coding(data_size_gb)
        
        report = f"\n{'='*80}\n"
        report += f"Performance Comparison: {data_size_gb}GB Dataset\n"
        report += f"{'='*80}\n\n"
        
        # Replication metrics
        report += str(repl) + "\n\n"
        
        # EC metrics
        report += str(ec) + "\n\n"
        
        # Comparison
        report += f"{'='*80}\n"
        report += "COMPARISON (EC vs Replication):\n"
        report += f"{'='*80}\n\n"
        
        # Storage efficiency
        storage_ratio = repl.storage_overhead / ec.storage_overhead
        report += f"Storage Efficiency:\n"
        report += f"  EC uses {storage_ratio:.1f}x LESS storage\n"
        report += f"  Savings: {repl.monthly_cost_dollars - ec.monthly_cost_dollars:,.0f}/month, "
        report += f"${(repl.monthly_cost_dollars - ec.monthly_cost_dollars)*12:,.0f}/year\n\n"
        
        # Performance ratios
        write_ratio = repl.write_latency_ms / ec.write_latency_ms
        read_ratio = repl.read_latency_ms / ec.read_latency_ms
        encode_cpu_ratio = ec.encode_cpu_percent / repl.encode_cpu_percent
        decode_cpu_ratio = ec.decode_cpu_percent / repl.decode_cpu_percent
        
        report += f"Write Performance:\n"
        report += f"  Replication is {write_ratio:.1f}x FASTER\n"
        report += f"  Latency difference: {ec.write_latency_ms - repl.write_latency_ms:.1f}ms\n\n"
        
        report += f"Read Performance:\n"
        report += f"  Replication is {read_ratio:.1f}x FASTER\n"
        report += f"  Latency difference: {ec.read_latency_ms - repl.read_latency_ms:.1f}ms\n\n"
        
        report += f"CPU Overhead:\n"
        report += f"  EC uses {encode_cpu_ratio:.1f}x MORE CPU for encoding\n"
        report += f"  EC uses {decode_cpu_ratio:.1f}x MORE CPU for decoding\n\n"
        
        # Network efficiency
        network_ratio = repl.network_io_gb / ec.network_io_gb
        report += f"Network I/O (Write):\n"
        report += f"  EC uses {network_ratio:.1f}x LESS network bandwidth\n"
        report += f"  Savings: {repl.network_io_gb - ec.network_io_gb:.2f}GB network I/O\n\n"
        
        # Rebuild performance
        rebuild_ratio = repl.rebuild_time_seconds / ec.rebuild_time_seconds
        report += f"Disk Failure Recovery:\n"
        report += f"  EC recovers {rebuild_ratio:.1f}x FASTER\n"
        report += f"  Time difference: {repl.rebuild_time_seconds - ec.rebuild_time_seconds:.2f}s\n\n"
        
        report += f"{'='*80}\n"
        
        return report


def main():
    """Run benchmarks for different dataset sizes"""
    sizes = [1, 10, 100, 1000]  # GB
    
    print("\n" + "="*80)
    print("DYNAMICFS PERFORMANCE COMPARISON: Replication vs Erasure Coding")
    print("="*80)
    
    for size in sizes:
        print(PerformanceBenchmark.compare_metrics(size))
    
    # Annual cost analysis for 1PB
    print("\nANNUAL COST ANALYSIS (1PB = 1000 TB Dataset):")
    print("="*80)
    
    data_size_1pb_gb = 1000 * 1024  # 1024 TB
    repl_1pb = PerformanceBenchmark.benchmark_replication(1)
    ec_1pb = PerformanceBenchmark.benchmark_erasure_coding(1)
    
    repl_annual = repl_1pb.monthly_cost_dollars * 1000 * 12
    ec_annual = ec_1pb.monthly_cost_dollars * 1000 * 12
    savings = repl_annual - ec_annual
    
    print(f"Replication (3x):     ${repl_annual:>15,.0f}/year")
    print(f"Erasure Coding (4+2): ${ec_annual:>15,.0f}/year")
    print(f"Annual Savings:       ${savings:>15,.0f}/year")
    print(f"Savings %:            {(savings/repl_annual)*100:>15.1f}%")
    print("\n")
    
    # Workload-specific recommendations
    print("\nWORKLOAD-SPECIFIC RECOMMENDATIONS:")
    print("="*80)
    
    recommendations = {
        "HOT (>100 ops/day)": {
            "strategy": "Replication (3x)",
            "reason": "Low latency critical for frequent access",
            "expected_reads": "50-100 per second"
        },
        "WARM (10-100 ops/day)": {
            "strategy": "Replication (3x) or Hybrid",
            "reason": "Balanced performance and cost",
            "expected_reads": "5-10 per second"
        },
        "COLD (<10 ops/day)": {
            "strategy": "Erasure Coding (4+2)",
            "reason": "Cost efficiency more important than latency",
            "expected_reads": "<1 per second"
        }
    }
    
    for tier, details in recommendations.items():
        print(f"\n{tier}:")
        print(f"  Strategy: {details['strategy']}")
        print(f"  Reason: {details['reason']}")
        print(f"  Throughput: {details['expected_reads']}")


if __name__ == "__main__":
    main()
