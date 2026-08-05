import psutil

class SystemMetrics:
    def get_cpu_percent(self) -> float:
        return psutil.cpu_percent(interval=None)

    def get_ram_used_mb(self) -> float:
        mem = psutil.virtual_memory()
        return mem.used / (1024 * 1024)

    def get_ram_total_mb(self) -> float:
        mem = psutil.virtual_memory()
        return mem.total / (1024 * 1024)

    def to_dict(self) -> dict:
        return {
            'cpu_usage_percent': self.get_cpu_percent(),
            'ram_used_mb': self.get_ram_used_mb(),
            'ram_total_mb': self.get_ram_total_mb()
        }
