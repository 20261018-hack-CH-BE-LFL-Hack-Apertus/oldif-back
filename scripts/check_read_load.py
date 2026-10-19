#!/usr/bin/env python3
"""Bounded read-only API concurrency check, not a production capacity benchmark."""
import argparse
import concurrent.futures
import datetime
import http.cookiejar
import json
import os
import platform
import statistics
import time
import urllib.error
import urllib.request
from pathlib import Path

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--base",required=True)
    parser.add_argument("--origin",required=True)
    parser.add_argument("--runtime-description",required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--requests",type=int,default=200)
    parser.add_argument("--workers",type=int,default=8)
    args=parser.parse_args()
    if not 1<=args.requests<=500 or not 1<=args.workers<=8:
        parser.error("Bound this acceptance test to 1-500 requests and 1-8 workers")
    jar=http.cookiejar.CookieJar()
    client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    request=urllib.request.Request(args.base.rstrip("/")+"/api/auth/demo",data=b"{}",headers={"Origin":args.origin,"Content-Type":"application/json"})
    with client.open(request,timeout=30) as response:
        assert response.status==200
        response.read()
    cookie="; ".join(c.name+"="+c.value for c in jar)
    def read(index):
        path=["/api/documents","/api/datasets"][index%2]
        start=time.monotonic(); result={"index":index,"path":path}
        req=urllib.request.Request(args.base.rstrip("/")+path,headers={"Origin":args.origin,"Cookie":cookie})
        try:
            with urllib.request.urlopen(req,timeout=30) as response:
                body=response.read(); decoded=json.loads(body)
                result.update(status=response.status,bytes=len(body),shape_valid=isinstance(decoded.get("documents" if path.endswith("documents") else "datasets"),list))
        except urllib.error.HTTPError as error:
            result.update(status=error.code,shape_valid=False)
        except (OSError,ValueError) as error:
            result.update(status=0,shape_valid=False,error=type(error).__name__)
        result["milliseconds"]=round((time.monotonic()-start)*1000,3)
        return result
    start=time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        rows=list(executor.map(read,range(args.requests)))
    elapsed=time.monotonic()-start
    latencies=sorted(row["milliseconds"] for row in rows)
    passed=sum(row["status"]==200 and row["shape_valid"] for row in rows)
    report={"observed_at":datetime.datetime.now(datetime.timezone.utc).isoformat(),"validation_class":"bounded concurrent authenticated API reads","runtime":args.runtime_description,"host":{"os":platform.system(),"architecture":platform.machine(),"logical_cpus":os.cpu_count()},"workers":args.workers,"requests":args.requests,"passed":passed,"failed":args.requests-passed,"elapsed_seconds":round(elapsed,3),"latency_ms":{"median":statistics.median(latencies),"p95":latencies[max(0,int(len(latencies)*.95)-1)],"max":max(latencies)},"requests_per_second":round(args.requests/elapsed,2),"provider_calls":0,"response_contents_retained":False,"production_capacity_claimed":False,"thousand_page_pipeline_claimed":False,"requests_detail":rows}
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({key:report[key] for key in ["passed","failed","elapsed_seconds","latency_ms","requests_per_second"]}))
    raise SystemExit(passed!=args.requests)

if __name__=="__main__":
    main()
