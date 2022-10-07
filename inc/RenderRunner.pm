package RenderRunner;

use strict;

use parent 'ProcessPipe';

use Term::ANSIColor qw(:constants);

# an object that can be used to render an image sequence into a video

sub new {
  my ($class,
      $ffmpeg_cmd,
      $output_dirname,
      $output_video_filename,
      $group,
      $start_callback,
      $successful_update_callback,
      $finished_callback,
      $group_finished_callback,
     )
      = @_;

  my $self = $class->SUPER::new($ffmpeg_cmd, $group,
				$start_callback,
				$successful_update_callback,
				$finished_callback,
				$group_finished_callback);
  $self->{output_dirname} = $output_dirname;
  $self->{output_video_filename} = $output_video_filename;
  $self->{frame_num} = 0;

  return bless $self, $class;
}

sub read_line($) {
 my ($self) = @_;

 my $ret = undef;
 if(eof($self->{shell_input})) {
   $self->finish();
 } else {
   $/ = ""; # don't use newline for <> XXX explore 'local' for this more
   my $line = readline($self->{shell_input});
   $/ = "\n";
   if (defined $line) {
     # frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x 
     if ($line =~ /^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/) {
       $self->{frame_num} = $1;
       $self->{fps} = $2;

       my $successful_update_callback = $self->{successful_update_callback};
       &$successful_update_callback($self) if defined $successful_update_callback;
     }
     $ret = 0;
   }
 }
 return $ret;
}

sub finish($) {
  my ($self) = @_;

  if ($self->output_video_exists()) {
    $self->{result} = 1;
  } else {
    # failed, why?
    $self->{result} = 'error';	# XXX expose errors somehow
  }

  $self->SUPER::finish();
}

sub output_video_exists() {
  my ($self) = @_;

  return -e $self->full_output_video_filename();
}

sub full_output_video_filename() {
  my ($self) = @_;

  return "$self->{output_dirname}/$self->{output_video_filename}";
}


1;
