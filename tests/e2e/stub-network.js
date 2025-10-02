const os = require('os');
os.networkInterfaces = () => ({
  lo: [
    {
      address: '127.0.0.1',
      family: 'IPv4',
      internal: true,
      netmask: '255.0.0.0',
      cidr: '127.0.0.1/8',
      mac: '00:00:00:00:00:00',
    },
  ],
});
