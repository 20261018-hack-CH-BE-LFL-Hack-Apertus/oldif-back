#!/usr/bin/env python3
"""Verify authenticated raw-WAV forwarding using an explicit local audio fixture."""
import argparse,hashlib,http.cookiejar,json,pathlib,time,urllib.request,urllib.error,wave
p=argparse.ArgumentParser();p.add_argument('--base',required=True);p.add_argument('--origin',required=True);p.add_argument('--wav',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
checks=[];agent=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
def call(path,data=None,headers=None,authenticated=True):
 h={'Origin':a.origin,**(headers or {})}
 if isinstance(data,dict):data=json.dumps(data).encode();h['Content-Type']='application/json'
 try:
  opener=agent if authenticated else urllib.request.build_opener()
  with opener.open(urllib.request.Request(a.base+path,data=data,headers=h),timeout=120) as r:return r.status,r.read()
 except urllib.error.HTTPError as e:return e.code,e.read()
def check(name,value):
 checks.append({'check':name,'passed':bool(value)})
 if not value:raise AssertionError(name)
audio=a.wav.read_bytes();headers={'Content-Type':'audio/wav','X-Local-Processing-Confirmed':'true'}
check('anonymous_audio_denied',call('/api/diarization',audio,headers,False)[0]==401)
check('demo_auth',call('/api/auth/demo',{})[0]==200)
check('capability_proxy',call('/api/diarization')[0]==200)
check('explicit_consent_required',call('/api/diarization',audio,{'Content-Type':'audio/wav'})[0]==400)
check('wrong_media_rejected',call('/api/diarization',audio,{'Content-Type':'application/octet-stream','X-Local-Processing-Confirmed':'true'})[0]==400)
check('oversized_audio_rejected',call('/api/diarization',b'x'*2_900_001,headers)[0]==413)
check('malformed_wav_rejected_by_core',call('/api/diarization',b'not a wave',headers)[0]==400)
started=time.monotonic();response=call('/api/diarization',audio,headers);elapsed=time.monotonic()-started
if response[0]!=200:print('diarization error',response)
check('actual_diarization_completed',response[0]==200);result=json.loads(response[1])
check('input_hash_preserved',result['input_sha256']==hashlib.sha256(audio).hexdigest())
check('no_identity_or_transcription_claim',result['identity_inference'] is False and result['transcription_performed'] is False)
with wave.open(str(a.wav)) as w:duration=w.getnframes()/w.getframerate()
check('segments_within_input_duration',all(0<=s['start']<s['end']<=duration+0.05 for s in result['segments']))
check('two_anonymous_speaker_labels',result['speaker_count']==2 and len({s['speaker'] for s in result['segments']})==2)
report={'validation_class':'actual local backend-to-core Nemotron3 CPU diarization on supplied synthetic two-speaker fixture','passed':sum(x['passed'] for x in checks),'failed':sum(not x['passed'] for x in checks),'checks':checks,'duration_seconds':duration,'analysis_seconds':round(elapsed,3),'result':result,'provider_calls':0,'real_human_speaker_accuracy':'NOT_RUN','audio_saved_by_backend':False}
a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'passed':report['passed'],'failed':report['failed'],'analysis_seconds':report['analysis_seconds'],'speaker_count':result['speaker_count']}))
