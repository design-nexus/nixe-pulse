"""FIDO2 hardware passkey bridge. Secrets and challenges are carried on stdin."""
import base64,ctypes as C,json,sys
L=C.CDLL('libfido2.so.1')
P=C.c_void_p; Z=C.c_size_t; I=C.c_int

def bind(name,result,args):
 f=getattr(L,name);f.restype=result;f.argtypes=args;return f
init=bind('fido_init',None,[I]);init(0)
info_new=bind('fido_dev_info_new',P,[Z]);manifest=bind('fido_dev_info_manifest',I,[P,Z,C.POINTER(Z)]);info_ptr=bind('fido_dev_info_ptr',P,[P,Z]);info_free=bind('fido_dev_info_free',None,[C.POINTER(P),Z])
def string(name,p):
 return (bind(name,C.c_char_p,[P])(p) or b'').decode('utf-8','replace')
def devices():
 p=P(info_new(64));count=Z()
 try:
  check(manifest(p,64,C.byref(count)))
  return [{'path':string('fido_dev_info_path',info_ptr(p,i)),'name':(string('fido_dev_info_manufacturer_string',info_ptr(p,i))+' '+string('fido_dev_info_product_string',info_ptr(p,i))).strip()} for i in range(min(count.value,64))]
 finally:info_free(C.byref(p),64)
def check(code):
 if code!=0:
  msg=bind('fido_strerr',C.c_char_p,[I])(code)
  raise RuntimeError((msg or b'Security key failed').decode())
def decode(s):return base64.urlsafe_b64decode(s+'='*(-len(s)%4))
def encode(b):return base64.b64encode(b).decode()
def data(name,p,index=None):
 args=[P] if index is None else [P,Z]
 size=bind(name+'_len',Z,args)(p,*(() if index is None else (index,)))
 ptr=bind(name+'_ptr',P,args)(p,*(() if index is None else (index,)))
 if size>1048576 or (size and not ptr):raise RuntimeError('Invalid security-key response')
 return C.string_at(ptr,size) if size else b''
def run(config):
 params=config['parameters'];params=params.get('publicKey',params)
 rp=params.get('rpId') or params.get('rp',{}).get('id') or 'telegram.org'
 if rp not in ('telegram.org','t.me'):raise ValueError('Unexpected passkey relying party')
 client=json.dumps({'type':'webauthn.create' if config['create'] else 'webauthn.get','challenge':params['challenge'],'origin':'https://'+rp,'crossOrigin':False},separators=(',',':')).encode()
 dev=P(bind('fido_dev_new',P,[])());obj=P()
 pin=config.get('pin','').encode() or None
 try:
  check(bind('fido_dev_open',I,[P,C.c_char_p])(dev,config['device'].encode()))
  check(bind('fido_dev_set_timeout',I,[P,I])(dev,60000))
  if config['create']:
   obj=P(bind('fido_cred_new',P,[])())
   check(bind('fido_cred_set_type',I,[P,I])(obj,-7))
   check(bind('fido_cred_set_clientdata',I,[P,P,Z])(obj,client,len(client)))
   check(bind('fido_cred_set_rp',I,[P,C.c_char_p,C.c_char_p])(obj,rp.encode(),params.get('rp',{}).get('name','Telegram').encode()))
   user=params['user'];uid=decode(user['id'])
   check(bind('fido_cred_set_user',I,[P,P,Z,C.c_char_p,C.c_char_p,C.c_char_p])(obj,uid,len(uid),user.get('name','Telegram').encode(),user.get('displayName','Telegram').encode(),None))
   check(bind('fido_cred_set_rk',I,[P,I])(obj,2))
   for credential in params.get('excludeCredentials',[]):
    raw=decode(credential['id']);check(bind('fido_cred_exclude',I,[P,P,Z])(obj,raw,len(raw)))
   check(bind('fido_dev_make_cred',I,[P,P,C.c_char_p])(dev,obj,pin))
   return {'client_data':client.decode(),'attestation_object':encode(data('fido_cred_attobj',obj))}
  obj=P(bind('fido_assert_new',P,[])())
  check(bind('fido_assert_set_rp',I,[P,C.c_char_p])(obj,rp.encode()))
  check(bind('fido_assert_set_clientdata',I,[P,P,Z])(obj,client,len(client)))
  check(bind('fido_assert_set_up',I,[P,I])(obj,2))
  for credential in params.get('allowCredentials',[]):
   raw=decode(credential['id']);check(bind('fido_assert_allow_cred',I,[P,P,Z])(obj,raw,len(raw)))
  check(bind('fido_dev_get_assert',I,[P,P,C.c_char_p])(dev,obj,pin))
  if bind('fido_assert_count',Z,[P])(obj)!=1:raise RuntimeError('Use a security key with a single matching Telegram passkey')
  return {'credential_id':base64.urlsafe_b64encode(data('fido_assert_id',obj,0)).decode().rstrip('='),'client_data':client.decode(),'authenticator_data':encode(data('fido_assert_authdata_raw',obj,0)),'signature':encode(data('fido_assert_sig',obj,0)),'user_handle':encode(data('fido_assert_user_id',obj,0))}
 finally:
  if obj:bind('fido_cred_free' if config['create'] else 'fido_assert_free',None,[C.POINTER(P)])(C.byref(obj))
  if dev:
   bind('fido_dev_close',I,[P])(dev);bind('fido_dev_free',None,[C.POINTER(P)])(C.byref(dev))
if __name__=='__main__':
 try:print(json.dumps({'result':devices() if '--devices' in sys.argv else run(json.loads(sys.stdin.readline(1048576)))}))
 except Exception as error:print(json.dumps({'error':str(error)}));sys.exit(1)
