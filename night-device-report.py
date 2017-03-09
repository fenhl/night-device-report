#!/usr/bin/env python3

if __name__ != '__main__':
    raise ImportError('This module is not for importing!')

import sys

sys.path.append('/opt/py')

import basedir
import platform
import requests

config = basedir.config_dirs('fenhl/night.json').json()
device_key = config['deviceKey']
hostname = platform.node().split('.')[0]

data = {}

response = requests.post('https://v2.nightd.fenhl.net/dev/{}/report'.format(hostname), json={'args': [device_key], 'data': data}, timeout=60.05)
response.raise_for_status()
j = response.json()
if 'text' in j:
    print(j['text'])
