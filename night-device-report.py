#!/usr/bin/env python3

if __name__ != '__main__':
    raise ImportError('This module is not for importing!')

import os.path
import sys

sys.path.append('/opt/py')
sys.path.append(os.path.expanduser('~/py'))

import basedir
import pathlib
import platform
import requests
import shutil
import subprocess

def night_excepthook(type, value, traceback):
    sys.__excepthook__(type, value, traceback)
    if issubclass(type, requests.RequestException) and value.response is not None:
        print('\n' + value.response.text, file=sys.stderr)

sys.excepthook = night_excepthook

CONFIG = basedir.config_dirs('fenhl/night.json').json()
DEVICE_KEY = CONFIG['deviceKey']
HOSTNAME = CONFIG.get('hostname', platform.node().split('.')[0])

data = {'key': DEVICE_KEY}

# cron-apt

data['cronApt'] = False
syslogs = [pathlib.Path('/var/log/syslog'), pathlib.Path('/var/log/syslog.1')]
for log_path in syslogs:
    if log_path.exists():
        with log_path.open('rb') as log_f:
            for line in reversed(list(log_f)):
                if b'cron-apt: Download complete and in download only mode' in line:
                    data['cronApt'] = True
                    break
                elif b'cron-apt: 0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.' in line:
                    data['cronApt'] = False
                    break
            else:
                continue
            break

# diskspace

usage = shutil.disk_usage('/')
data['diskspaceTotal'] = usage.total
data['diskspaceFree'] = usage.free

# needrestart

try:
    for line in subprocess.check_output(['/usr/sbin/needrestart', '-b'], stderr=subprocess.DEVNULL).decode('utf-8').split('\n'):
        if line.startswith('NEEDRESTART-KSTA: '):
            data['needrestart'] = int(line[len('NEEDRESTART-KSTA: '):])
            break
    else:
        data['needrestart'] = None
except FileNotFoundError:
    data['needrestart'] = None

# send data

response = requests.post('https://nightd.fenhl.net/device-report/{}'.format(HOSTNAME), json=data, timeout=600)
response.raise_for_status()
