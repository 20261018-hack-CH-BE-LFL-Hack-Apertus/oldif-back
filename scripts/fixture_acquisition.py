"""Acquisition metadata for explicitly authorized local test fixtures; no rights verification claim."""
import datetime, hashlib

def acquisition(content, rights_identifier, provider=False):
    digest=hashlib.sha256(content).hexdigest()
    uses=['local_processing','private_export']
    if provider: uses.append('provider_processing')
    return {'dataset_uri':'urn:oldif:local-fixture:'+digest,'distribution_uri':'urn:sha256:'+digest,'retrieved_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'sha256':digest,'rights_identifier':rights_identifier,'allowed_uses':uses}
