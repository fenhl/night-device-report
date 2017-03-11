#!/usr/bin/env python3

if __name__ != '__main__':
    raise ImportError('This module is not for importing!')

import sys

sys.path.append('/opt/py')

import basedir
import platform
import requests
import subprocess

def night_excepthook(type, value, traceback):
    sys.__excepthook__(type, value, traceback)
    if issubclass(type, requests.RequestException) and value.response is not None:
        print('\n' + value.response.text, file=sys.stderr)

sys.excepthook = night_excepthook

CONFIG = basedir.config_dirs('fenhl/night.json').json()
DEVICE_KEY = CONFIG['deviceKey']
HOSTNAME = CONFIG.get('hostname', platform.node().split('.')[0])

data = {}

for line in subprocess.check_output(['/usr/sbin/needrestart', '-b'], stderr=subprocess.DEVNULL).decode('utf-8').split('\n'):
    if line.startswith('NEEDRESTART-KSTA: '):
        data['needrestart'] = int(line[len('NEEDRESTART-KSTA: '):])
        break
else:
    data['needrestart'] = None

response = requests.post('https://v2.nightd.fenhl.net/dev/{}/report'.format(HOSTNAME), json={'args': [DEVICE_KEY], 'data': data}, timeout=60.05)
response.raise_for_status()
j = response.json()
if 'text' in j:
    print(j['text'])
