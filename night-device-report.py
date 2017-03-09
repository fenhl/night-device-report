#!/usr/bin/env python3

if __name__ != '__main__':
    raise ImportError('This module is not for importing!')

import sys

sys.path.append('/opt/py')

import basedir
import platform
import requests
import subprocess

def which(cmd):
    try:
        return subprocess.check_output(['which', cmd], stderr=subprocess.DEVNULL).decode('utf-8')[:-1]
    except subprocess.CalledProcessError:
        return None

config = basedir.config_dirs('fenhl/night.json').json()
device_key = config['deviceKey']
hostname = platform.node().split('.')[0]

data = {}

if which('needrestart') is not None:
    for line in subprocess.check_output(['needrestart', '-b'], stderr=subprocess.DEVNULL).decode('utf-8').split('\n'):
        if line.startswith('NEEDRESTART-KSTA: '):
            data['needrestart'] = int(line[len('NEEDRESTART-KSTA: '):])

response = requests.post('https://v2.nightd.fenhl.net/dev/{}/report'.format(hostname), json={'args': [device_key], 'data': data}, timeout=60.05)
response.raise_for_status()
j = response.json()
if 'text' in j:
    print(j['text'])
