#!/usr/bin/env python3
"""
RAG Enterprise Benchmark Script
===============================
Standardized benchmark for comparing RAG performance across different hardware.

Usage:
    python rag_benchmark.py --api-url http://localhost:8000 --output ./results

Documents: Public domain with stable URLs for reproducible testing.
"""

import os
import sys
import json
import time
import argparse
import platform
import subprocess
import hashlib
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional
import requests
import statistics

# ============================================================================
# BENCHMARK DOCUMENTS - Public domain with stable URLs
# ============================================================================

BENCHMARK_DOCUMENTS = [
    # LARGE DOCUMENTS (stress test)
    {
        "id": "mueller_report",
        "name": "Mueller Report (2019)",
        "url": "https://www.justice.gov/archives/sco/file/1373816/dl",
        "type": "legal_large",
        "expected_pages": 448,
        "queries": [
            "What specific actions did the IRA take on Facebook?",
            "Who were the members of the Internet Research Agency leadership?",
            "What were the 10 episodes of potential obstruction?",
            "What was the role of Paul Manafort?",
        ]
    },
    {
        "id": "911_commission_report",
        "name": "9/11 Commission Report",
        "url": "https://www.9-11commission.gov/report/911Report.pdf",
        "type": "legal_large",
        "expected_pages": 585,
        "queries": [
            "What time did Flight 11 hit the North Tower?",
            "Who was the leader of the 9/11 hijackers?",
            "What were the main failures identified by the commission?",
            "What recommendations did the commission make?",
        ]
    },
    # MEDIUM DOCUMENTS
    {
        "id": "bitcoin_whitepaper",
        "name": "Bitcoin Whitepaper",
        "url": "https://bitcoin.org/bitcoin.pdf",
        "type": "technical",
        "expected_pages": 9,
        "queries": [
            "Who is the author of the Bitcoin whitepaper?",
            "What problem does Bitcoin solve?",
            "What is proof-of-work?",
        ]
    },
    {
        "id": "attention_paper",
        "name": "Attention Is All You Need (Transformers)",
        "url": "https://arxiv.org/pdf/1706.03762.pdf",
        "type": "technical",
        "expected_pages": 15,
        "queries": [
            "What is the main contribution of this paper?",
            "How many attention heads are used?",
            "What is multi-head attention?",
        ]
    },
    {
        "id": "gdpr_regulation",
        "name": "GDPR Full Text",
        "url": "https://eur-lex.europa.eu/legal-content/EN/TXT/PDF/?uri=CELEX:32016R0679",
        "type": "legal",
        "expected_pages": 88,
        "queries": [
            "What is the right to be forgotten?",
            "What are the penalties for GDPR violations?",
            "What is a Data Protection Officer?",
        ]
    },
]

# ============================================================================
# HARDWARE INFO COLLECTION
# ============================================================================

def get_hardware_info() -> Dict:
    """Collect hardware and software information"""
    info = {
        "timestamp": datetime.now().isoformat(),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "version": platform.version(),
            "machine": platform.machine(),
            "processor": platform.processor(),
        },
        "python_version": platform.python_version(),
    }

    # CPU info
    try:
        if platform.system() == "Linux":
            with open("/proc/cpuinfo", "r") as f:
                cpuinfo = f.read()
                for line in cpuinfo.split("\n"):
                    if "model name" in line:
                        info["cpu_model"] = line.split(":")[1].strip()
                        break

            # CPU cores
            info["cpu_cores"] = os.cpu_count()
    except:
        info["cpu_model"] = "Unknown"
        info["cpu_cores"] = os.cpu_count()

    # RAM info
    try:
        if platform.system() == "Linux":
            with open("/proc/meminfo", "r") as f:
                meminfo = f.read()
                for line in meminfo.split("\n"):
                    if "MemTotal" in line:
                        mem_kb = int(line.split()[1])
                        info["ram_gb"] = round(mem_kb / 1024 / 1024, 1)
                        break
    except:
        info["ram_gb"] = "Unknown"

    # GPU info (NVIDIA) - try multiple methods
    gpu_detected = False

    # Method 1: nvidia-smi with full path options
    nvidia_smi_paths = ["nvidia-smi", "/usr/bin/nvidia-smi", "/usr/local/bin/nvidia-smi"]
    for nvidia_smi in nvidia_smi_paths:
        try:
            result = subprocess.run(
                [nvidia_smi, "--query-gpu=name,memory.total,driver_version,cuda_version",
                 "--format=csv,noheader,nounits"],
                capture_output=True, text=True, timeout=10
            )
            if result.returncode == 0 and result.stdout.strip():
                parts = result.stdout.strip().split(", ")
                info["gpu"] = {
                    "name": parts[0].strip() if len(parts) > 0 else "Unknown",
                    "memory_mb": int(float(parts[1].strip())) if len(parts) > 1 else 0,
                    "driver_version": parts[2].strip() if len(parts) > 2 else "Unknown",
                    "cuda_version": parts[3].strip() if len(parts) > 3 else "Unknown",
                }
                gpu_detected = True
                break
        except:
            continue

    # Method 2: Try torch if nvidia-smi failed
    if not gpu_detected:
        try:
            import torch
            if torch.cuda.is_available():
                info["gpu"] = {
                    "name": torch.cuda.get_device_name(0),
                    "memory_mb": torch.cuda.get_device_properties(0).total_memory // (1024*1024),
                    "driver_version": "Unknown (detected via PyTorch)",
                    "cuda_version": torch.version.cuda or "Unknown",
                }
                gpu_detected = True
        except:
            pass

    if not gpu_detected:
        info["gpu"] = {"name": "No GPU detected"}

    return info


# ============================================================================
# BENCHMARK RUNNER
# ============================================================================

class RAGBenchmark:
    def __init__(self, api_url: str, output_dir: str, username: str = "admin", password: str = "admin"):
        self.api_url = api_url.rstrip("/")
        self.output_dir = Path(output_dir)
        self.output_dir.mkdir(parents=True, exist_ok=True)
        self.username = username
        self.password = password
        self.token = None
        self.results = {
            "benchmark_version": "1.0.0",
            "timestamp": datetime.now().isoformat(),
            "hardware": get_hardware_info(),
            "documents": [],
            "queries": [],
            "summary": {},
        }

    def authenticate(self) -> bool:
        """Authenticate with the RAG API"""
        try:
            response = requests.post(
                f"{self.api_url}/api/auth/login",
                json={"username": self.username, "password": self.password},
                timeout=30
            )
            if response.status_code == 200:
                self.token = response.json().get("access_token")
                print(f"✅ Authenticated as {self.username}")
                return True
            else:
                print(f"❌ Authentication failed: {response.status_code} - {response.text[:100]}")
                return False
        except Exception as e:
            print(f"❌ Authentication error: {e}")
            return False

    def get_headers(self) -> Dict:
        """Get headers with authentication"""
        headers = {"Content-Type": "application/json"}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers

    def download_document(self, doc: Dict, cache_dir: Path) -> Optional[Path]:
        """Download document if not cached"""
        cache_dir.mkdir(parents=True, exist_ok=True)
        filename = f"{doc['id']}.pdf"
        filepath = cache_dir / filename

        if filepath.exists():
            print(f"   📄 Using cached: {filename}")
            return filepath

        urls = [doc["url"]]
        if "backup_url" in doc:
            urls.append(doc["backup_url"])

        for url in urls:
            try:
                print(f"   ⬇️  Downloading from {url[:50]}...")
                response = requests.get(url, timeout=60)
                if response.status_code == 200:
                    filepath.write_bytes(response.content)
                    print(f"   ✅ Downloaded: {filename} ({len(response.content) / 1024:.1f} KB)")
                    return filepath
            except Exception as e:
                print(f"   ⚠️  Failed: {e}")
                continue

        print(f"   ❌ Could not download {doc['name']}")
        return None

    def upload_document(self, filepath: Path) -> Dict:
        """Upload document and measure time"""
        start_time = time.time()

        try:
            with open(filepath, "rb") as f:
                files = {"file": (filepath.name, f, "application/pdf")}
                headers = {}
                if self.token:
                    headers["Authorization"] = f"Bearer {self.token}"

                response = requests.post(
                    f"{self.api_url}/api/documents/upload",
                    files=files,
                    headers=headers,
                    timeout=300  # 5 min timeout for large docs
                )

            upload_time = time.time() - start_time

            if response.status_code in [200, 202]:
                result = response.json()
                return {
                    "success": True,
                    "upload_time_seconds": round(upload_time, 2),
                    "document_id": result.get("document_id"),
                    "filename": result.get("filename"),
                    "file_size_kb": filepath.stat().st_size / 1024,
                }
            else:
                return {
                    "success": False,
                    "error": f"HTTP {response.status_code}: {response.text[:200]}",
                    "upload_time_seconds": round(upload_time, 2),
                }
        except Exception as e:
            return {
                "success": False,
                "error": str(e),
                "upload_time_seconds": time.time() - start_time,
            }

    def run_query(self, query: str, top_k: int = 15) -> Dict:
        """Run a query and measure time"""
        start_time = time.time()

        try:
            response = requests.post(
                f"{self.api_url}/api/query",
                json={"query": query, "top_k": top_k, "temperature": 0.7},
                headers=self.get_headers(),
                timeout=120
            )

            query_time = time.time() - start_time

            if response.status_code == 200:
                result = response.json()
                return {
                    "success": True,
                    "query": query,
                    "query_time_seconds": round(query_time, 2),
                    "answer_length": len(result.get("answer", "")),
                    "sources_count": len(result.get("sources", [])),
                    "top_similarity": result["sources"][0]["similarity_score"] if result.get("sources") else 0,
                }
            else:
                return {
                    "success": False,
                    "query": query,
                    "error": f"HTTP {response.status_code}",
                    "query_time_seconds": round(query_time, 2),
                }
        except Exception as e:
            return {
                "success": False,
                "query": query,
                "error": str(e),
                "query_time_seconds": time.time() - start_time,
            }

    def wait_for_indexing(self, expected_docs: int, timeout: int = 300) -> bool:
        """Wait for documents to be indexed"""
        print(f"   ⏳ Waiting for indexing (timeout: {timeout}s)...")
        start = time.time()

        while time.time() - start < timeout:
            try:
                response = requests.get(
                    f"{self.api_url}/api/documents",
                    headers=self.get_headers(),
                    timeout=30
                )
                if response.status_code == 200:
                    docs = response.json().get("documents", [])
                    if len(docs) >= expected_docs:
                        print(f"   ✅ {len(docs)} documents indexed")
                        return True
            except:
                pass
            time.sleep(2)

        print(f"   ⚠️  Timeout waiting for indexing")
        return False

    def clear_documents(self):
        """Clear all documents before benchmark"""
        try:
            response = requests.get(
                f"{self.api_url}/api/documents",
                headers=self.get_headers(),
                timeout=30
            )
            if response.status_code == 200:
                docs = response.json().get("documents", [])
                for doc in docs:
                    doc_id = doc.get("document_id")
                    if doc_id:
                        requests.delete(
                            f"{self.api_url}/api/documents/{doc_id}",
                            headers=self.get_headers(),
                            timeout=30
                        )
                print(f"   🗑️  Cleared {len(docs)} existing documents")
        except Exception as e:
            print(f"   ⚠️  Could not clear documents: {e}")

    def run(self, clear_first: bool = True):
        """Run the complete benchmark"""
        print("=" * 70)
        print("RAG ENTERPRISE BENCHMARK")
        print("=" * 70)
        print(f"API URL: {self.api_url}")
        print(f"Output: {self.output_dir}")
        print()

        # Authenticate
        if not self.authenticate():
            print("❌ Cannot proceed without authentication")
            return

        # Clear existing documents
        if clear_first:
            print("\n📋 Clearing existing documents...")
            self.clear_documents()

        # Download and upload documents
        print("\n📥 PHASE 1: Document Upload")
        print("-" * 40)

        cache_dir = self.output_dir / "document_cache"
        uploaded_count = 0

        for doc in BENCHMARK_DOCUMENTS:
            print(f"\n📄 {doc['name']} ({doc['type']})")

            filepath = self.download_document(doc, cache_dir)
            if not filepath:
                continue

            upload_result = self.upload_document(filepath)
            upload_result["document_name"] = doc["name"]
            upload_result["document_type"] = doc["type"]
            self.results["documents"].append(upload_result)

            if upload_result["success"]:
                uploaded_count += 1
                print(f"   ✅ Uploaded in {upload_result['upload_time_seconds']}s")
            else:
                print(f"   ❌ Failed: {upload_result.get('error')}")

        # Wait for indexing
        print("\n⏳ Waiting for indexing to complete...")
        self.wait_for_indexing(uploaded_count, timeout=300)

        # Run queries
        print("\n❓ PHASE 2: Query Benchmark")
        print("-" * 40)

        all_query_times = []

        for doc in BENCHMARK_DOCUMENTS:
            print(f"\n📄 Queries for: {doc['name']}")

            for query in doc["queries"]:
                print(f"   Q: {query[:50]}...")
                result = self.run_query(query)
                result["document_name"] = doc["name"]
                self.results["queries"].append(result)

                if result["success"]:
                    all_query_times.append(result["query_time_seconds"])
                    print(f"      ✅ {result['query_time_seconds']}s | "
                          f"Similarity: {result['top_similarity']:.2%} | "
                          f"Sources: {result['sources_count']}")
                else:
                    print(f"      ❌ {result.get('error')}")

        # Calculate summary
        print("\n📊 PHASE 3: Calculating Summary")
        print("-" * 40)

        successful_uploads = [d for d in self.results["documents"] if d.get("success")]
        successful_queries = [q for q in self.results["queries"] if q.get("success")]

        upload_times = [d["upload_time_seconds"] for d in successful_uploads]
        query_times = [q["query_time_seconds"] for q in successful_queries]

        self.results["summary"] = {
            "total_documents": len(BENCHMARK_DOCUMENTS),
            "successful_uploads": len(successful_uploads),
            "failed_uploads": len(self.results["documents"]) - len(successful_uploads),
            "total_queries": len(self.results["queries"]),
            "successful_queries": len(successful_queries),
            "failed_queries": len(self.results["queries"]) - len(successful_queries),
            "upload_stats": {
                "mean_seconds": round(statistics.mean(upload_times), 2) if upload_times else 0,
                "median_seconds": round(statistics.median(upload_times), 2) if upload_times else 0,
                "min_seconds": round(min(upload_times), 2) if upload_times else 0,
                "max_seconds": round(max(upload_times), 2) if upload_times else 0,
            },
            "query_stats": {
                "mean_seconds": round(statistics.mean(query_times), 2) if query_times else 0,
                "median_seconds": round(statistics.median(query_times), 2) if query_times else 0,
                "min_seconds": round(min(query_times), 2) if query_times else 0,
                "max_seconds": round(max(query_times), 2) if query_times else 0,
                "p95_seconds": round(sorted(query_times)[int(len(query_times) * 0.95)] if len(query_times) >= 20 else max(query_times), 2) if query_times else 0,
            },
        }

        # Save results
        self.save_results()

        # Print summary
        self.print_summary()

    def save_results(self):
        """Save results to JSON and Markdown"""
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")

        # JSON
        json_path = self.output_dir / f"benchmark_{timestamp}.json"
        with open(json_path, "w") as f:
            json.dump(self.results, f, indent=2)
        print(f"\n💾 Results saved: {json_path}")

        # Markdown report
        md_path = self.output_dir / f"benchmark_{timestamp}.md"
        self.generate_markdown_report(md_path)
        print(f"📄 Report saved: {md_path}")

    def generate_markdown_report(self, filepath: Path):
        """Generate Markdown report"""
        hw = self.results["hardware"]
        summary = self.results["summary"]

        report = f"""# RAG Enterprise Benchmark Report

**Date:** {self.results['timestamp']}
**Benchmark Version:** {self.results['benchmark_version']}

## Hardware Configuration

| Component | Details |
|-----------|---------|
| **CPU** | {hw.get('cpu_model', 'Unknown')} ({hw.get('cpu_cores', '?')} cores) |
| **RAM** | {hw.get('ram_gb', '?')} GB |
| **GPU** | {hw.get('gpu', {}).get('name', 'None')} |
| **GPU Memory** | {hw.get('gpu', {}).get('memory_mb', 0)} MB |
| **CUDA Version** | {hw.get('gpu', {}).get('cuda_version', 'N/A')} |
| **OS** | {hw.get('platform', {}).get('system', '?')} {hw.get('platform', {}).get('release', '')} |
| **Python** | {hw.get('python_version', '?')} |

## Summary

| Metric | Value |
|--------|-------|
| **Documents Uploaded** | {summary.get('successful_uploads', 0)}/{summary.get('total_documents', 0)} |
| **Queries Executed** | {summary.get('successful_queries', 0)}/{summary.get('total_queries', 0)} |

### Upload Performance

| Metric | Time (seconds) |
|--------|----------------|
| Mean | {summary.get('upload_stats', {}).get('mean_seconds', 0)} |
| Median | {summary.get('upload_stats', {}).get('median_seconds', 0)} |
| Min | {summary.get('upload_stats', {}).get('min_seconds', 0)} |
| Max | {summary.get('upload_stats', {}).get('max_seconds', 0)} |

### Query Performance

| Metric | Time (seconds) |
|--------|----------------|
| Mean | {summary.get('query_stats', {}).get('mean_seconds', 0)} |
| Median | {summary.get('query_stats', {}).get('median_seconds', 0)} |
| Min | {summary.get('query_stats', {}).get('min_seconds', 0)} |
| Max | {summary.get('query_stats', {}).get('max_seconds', 0)} |
| P95 | {summary.get('query_stats', {}).get('p95_seconds', 0)} |

## Document Details

| Document | Type | Upload Time (s) | Status |
|----------|------|-----------------|--------|
"""
        for doc in self.results["documents"]:
            status = "✅" if doc.get("success") else "❌"
            report += f"| {doc.get('document_name', 'Unknown')} | {doc.get('document_type', '?')} | {doc.get('upload_time_seconds', 0)} | {status} |\n"

        report += """
## Query Details

| Document | Query | Time (s) | Similarity | Status |
|----------|-------|----------|------------|--------|
"""
        for q in self.results["queries"]:
            status = "✅" if q.get("success") else "❌"
            similarity = f"{q.get('top_similarity', 0):.1%}" if q.get("success") else "N/A"
            query_short = q.get('query', '')[:40] + "..." if len(q.get('query', '')) > 40 else q.get('query', '')
            report += f"| {q.get('document_name', '?')} | {query_short} | {q.get('query_time_seconds', 0)} | {similarity} | {status} |\n"

        report += """
---
*Generated by RAG Enterprise Benchmark*
"""

        filepath.write_text(report)

    def print_summary(self):
        """Print summary to console"""
        summary = self.results["summary"]
        hw = self.results["hardware"]

        print("\n" + "=" * 70)
        print("BENCHMARK SUMMARY")
        print("=" * 70)
        print(f"Hardware: {hw.get('cpu_model', 'Unknown')}")
        print(f"GPU: {hw.get('gpu', {}).get('name', 'None')}")
        print(f"RAM: {hw.get('ram_gb', '?')} GB")
        print("-" * 70)
        print(f"Documents: {summary.get('successful_uploads', 0)}/{summary.get('total_documents', 0)} uploaded")
        print(f"Queries: {summary.get('successful_queries', 0)}/{summary.get('total_queries', 0)} successful")
        print("-" * 70)
        print(f"Upload Mean: {summary.get('upload_stats', {}).get('mean_seconds', 0)}s")
        print(f"Query Mean: {summary.get('query_stats', {}).get('mean_seconds', 0)}s")
        print(f"Query P95: {summary.get('query_stats', {}).get('p95_seconds', 0)}s")
        print("=" * 70)


# ============================================================================
# MAIN
# ============================================================================

def main():
    parser = argparse.ArgumentParser(description="RAG Enterprise Benchmark")
    parser.add_argument("--api-url", default="http://localhost:8000",
                        help="RAG API URL (default: http://localhost:8000)")
    parser.add_argument("--output", default="./benchmark_results",
                        help="Output directory (default: ./benchmark_results)")
    parser.add_argument("--username", default="admin",
                        help="API username (default: admin)")
    parser.add_argument("--password", default="admin",
                        help="API password (default: admin)")
    parser.add_argument("--no-clear", action="store_true",
                        help="Don't clear existing documents before benchmark")

    args = parser.parse_args()

    benchmark = RAGBenchmark(
        api_url=args.api_url,
        output_dir=args.output,
        username=args.username,
        password=args.password,
    )

    benchmark.run(clear_first=not args.no_clear)


if __name__ == "__main__":
    main()
