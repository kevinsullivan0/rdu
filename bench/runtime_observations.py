"""Future opt-in read-only host observations, outside per-scanner timing."""
import subprocess
import time

COMMANDS=(('uptime',['/usr/bin/uptime']),('battery',['/usr/bin/pmset','-g','batt']),
          ('thermal_status',['/usr/bin/pmset','-g','therm']))


def capture_runtime_observations(deadline):
    result={'schema':1,'recorded_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
            'read_only':True,'interpretation':'Bounded OS reports only; unknowns retained. No CPUclock, thermalfloor or physical-optimum inference.',
            'observations':{}}
    for label,command in COMMANDS:
        remaining=deadline-time.monotonic()
        item={'command':command,'status':'unknown'}
        if remaining<=0:
            item['reason']='campaign deadline already exhausted'
        else:
            try:
                child=subprocess.run(command,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE,timeout=min(2,remaining),check=False)
                item.update(returncode=child.returncode,stdout=child.stdout[:4096].decode('utf8','backslashreplace'),
                            stderr=child.stderr[:1024].decode('utf8','backslashreplace'))
                item['truncated']=len(child.stdout)>4096 or len(child.stderr)>1024
                if child.returncode==0 and not item['truncated']:item['status']='observed'
                else:item['reason']='nonzero exit or output bound; report is incomplete'
            except FileNotFoundError:item['reason']='declared OS command unavailable'
            except subprocess.TimeoutExpired:item['reason']='two-second/remaining deadline; owned observer child killed by subprocess.run'
            except OSError as error:item['reason']='OS error';item['errno']=error.errno
        result['observations'][label]=item
    return result
